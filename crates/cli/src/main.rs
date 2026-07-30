use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};

/// Whether this binary was compiled with the optional `ocr` feature (the
/// Tesseract OCR backend). Reported by `--version` so a user can tell, without
/// running an `--ocr` command, whether OCR is available.
#[cfg(feature = "ocr")]
const OCR_COMPILED_IN: bool = true;
#[cfg(not(feature = "ocr"))]
const OCR_COMPILED_IN: bool = false;

/// The long `--version` string: CLI version, the underlying engine version, and
/// the compiled feature flags (currently just OCR). Built once at first use.
fn long_version() -> &'static str {
    use std::sync::OnceLock;
    static V: OnceLock<String> = OnceLock::new();
    V.get_or_init(|| {
        format!(
            "{cli}\nengine: {engine}\nocr: {ocr}\nfeatures: [{features}]",
            cli = env!("CARGO_PKG_VERSION"),
            engine = wellfriendpdf_engine::ENGINE_VERSION,
            ocr = if OCR_COMPILED_IN {
                "compiled-in (Tesseract backend available)"
            } else {
                "not compiled-in (rebuild with --features ocr to enable)"
            },
            features = if OCR_COMPILED_IN { "ocr" } else { "" },
        )
    })
    .as_str()
}

#[derive(Parser)]
#[command(
    name = "wellfriendpdf",
    about = "Wellfriend — pure-Rust PDF processing tool",
    version,
    long_version = long_version(),
    after_help = "Command groups:\n  Extraction: extract-text, extract-tables, extract-fields, extract-images, parse, document-model, chunk\n  Rendering/conversion: render, render-compare, pdf-to-jpg, image-to-pdf, pdf-to-xlsx, pdf-to-pptx, pdf-to-docx, xlsx-to-pdf, pptx-to-pdf, docx-to-pdf, to-html\n  Structure/editing: merge, split, extract-pages, organize, rotate, watermark, add-page-numbers, optimize, repair, linearize\n  Info/security: info, parser-report, security-report, signature-report, sanitize, validate, canonicalize, fonts, detach, verify-sig, encrypt, decrypt, analyze, eval-score\n\nExamples:\n  wellfriendpdf extract-text input.pdf --structured --format json\n  wellfriendpdf parser-report input.pdf --mode audit\n  wellfriendpdf pdf-to-jpg input.pdf --out-dir pages --dpi 150\n  wellfriendpdf image-to-pdf img1.jpg img2.png --out combined.pdf\n  wellfriendpdf pdf-to-xlsx report.pdf --out report.xlsx\n  wellfriendpdf pdf-to-pptx deck.pdf --out deck.pptx\n  wellfriendpdf pdf-to-docx report.pdf --out report.docx\n  wellfriendpdf xlsx-to-pdf workbook.xlsx --out workbook.pdf\n  wellfriendpdf watermark input.pdf --text CONFIDENTIAL --out out.pdf"
)]
struct Cli {
    /// Public execution mode. Standard is the default; Research enables only
    /// operator-configured optional accelerators/providers and falls back safely.
    #[arg(long, global = true, value_name = "standard|research")]
    mode: Option<String>,
    /// Runtime configuration file (JSON or the documented TOML-like subset).
    #[arg(long = "runtime-config-file", global = true, value_name = "PATH")]
    runtime_config_file: Option<PathBuf>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum CliExitCode {
    Success = 0,
    Internal = 1,
    Usage = 2,
    Io = 3,
    Input = 4,
    Unsupported = 5,
    /// A parsed signature failed its mathematical/CMS integrity checks.
    SignatureInvalid = 10,
    /// The signature was mathematically valid but did not establish trust
    /// under the caller-selected policy.
    Untrusted = 11,
    /// Authenticated revocation evidence reported a revoked certificate.
    Revoked = 12,
    /// Required validation evidence was missing, stale, malformed, or
    /// otherwise could not establish a policy decision.
    Indeterminate = 13,
    /// Controlled retrieval was allowed but failed before required evidence
    /// could be established.
    Network = 14,
}

impl CliExitCode {
    const fn code(self) -> u8 {
        self as u8
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Internal => "internal error",
            Self::Usage => "usage error",
            Self::Io => "I/O error",
            Self::Input => "parse/format error",
            Self::Unsupported => "unsupported feature",
            Self::SignatureInvalid => "signature invalid",
            Self::Untrusted => "signature untrusted",
            Self::Revoked => "certificate revoked",
            Self::Indeterminate => "signature validation indeterminate",
            Self::Network => "signature evidence network failure",
        }
    }
}

#[derive(Debug)]
struct CliError {
    kind: CliExitCode,
    message: String,
}

impl CliError {
    fn new(kind: CliExitCode, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for CliError {}

#[derive(Parser)]
struct RuntimeReportArgs {
    /// Write JSON to this path instead of stdout
    #[arg(long)]
    output: Option<PathBuf>,
}

#[derive(Parser)]
struct RuntimeConfigArgs {
    /// Emit the immutable effective configuration after host and admin policy
    #[arg(long)]
    effective: bool,
    /// Validate the requested configuration and report the effective mode
    #[arg(long)]
    validate: bool,
    /// Write JSON to this path instead of stdout
    #[arg(long)]
    output: Option<PathBuf>,
}

#[derive(Parser)]
struct ProvidersArgs {
    #[command(subcommand)]
    command: ProviderCommand,
    /// Write JSON to this path instead of stdout
    #[arg(long)]
    output: Option<PathBuf>,
}

#[derive(Subcommand)]
enum ProviderCommand {
    /// List provider contracts and configured availability
    List,
    /// Validate provider configuration and secret-hygiene posture
    Check,
}

fn usage_error(message: impl Into<String>) -> Box<dyn Error> {
    Box::new(CliError::new(CliExitCode::Usage, message))
}

fn unsupported_error(message: impl Into<String>) -> Box<dyn Error> {
    Box::new(CliError::new(CliExitCode::Unsupported, message))
}

fn classify_error(err: &(dyn Error + 'static)) -> CliExitCode {
    let mut current = Some(err);
    while let Some(error) = current {
        if let Some(cli) = error.downcast_ref::<CliError>() {
            return cli.kind;
        }
        if let Some(wellfriendpdf) = error.downcast_ref::<wellfriendpdf_engine::WellfriendError>() {
            return match wellfriendpdf.kind() {
                wellfriendpdf_engine::ErrorKind::Io => CliExitCode::Io,
                wellfriendpdf_engine::ErrorKind::UnsupportedFeature => CliExitCode::Unsupported,
                wellfriendpdf_engine::ErrorKind::MalformedPdf
                | wellfriendpdf_engine::ErrorKind::Parse
                | wellfriendpdf_engine::ErrorKind::MissingObject
                | wellfriendpdf_engine::ErrorKind::Encrypted
                | wellfriendpdf_engine::ErrorKind::AuthenticationFailure
                | wellfriendpdf_engine::ErrorKind::ResourceLimit => CliExitCode::Input,
                wellfriendpdf_engine::ErrorKind::Cancelled => CliExitCode::Internal,
            };
        }
        if error.downcast_ref::<std::io::Error>().is_some() {
            return CliExitCode::Io;
        }
        current = error.source();
    }

    let message = err.to_string().to_lowercase();
    if message.contains("unsupported")
        || message.contains("does not support")
        || message.contains("no ocr backend")
        || message.contains("rebuild the cli with")
    {
        CliExitCode::Unsupported
    } else if message.contains("unknown --")
        || message.contains("unknown format")
        || message.contains("unknown render quality")
        || message.contains("unknown --format")
        || message.contains("unknown --type")
        || message.contains("out of range")
        || message.contains("page range")
        || message.contains("mutually exclusive")
        || message.contains("cannot be combined")
        || message.contains("no pages selected")
        || message.contains("matched no pages")
    {
        CliExitCode::Usage
    } else {
        CliExitCode::Internal
    }
}

#[derive(Subcommand)]
enum Commands {
    /// Report runtime capabilities for Standard and Research modes
    Capabilities(RuntimeReportArgs),
    /// Validate or display the effective runtime configuration
    RuntimeConfig(RuntimeConfigArgs),
    /// List or check configured OCR/provider backends
    Providers(ProvidersArgs),
    /// Extract plain text from a PDF
    ExtractText(ExtractTextArgs),
    /// Detect and extract tables from a PDF as CSV or JSON (no Poppler equivalent)
    ExtractTables(ExtractTablesArgs),
    /// Parse a PDF into the canonical document model and serialize it to clean,
    /// structured Markdown / JSON / HTML for AI/RAG pipelines and data
    /// automation — headings/paragraphs/lists/tables/figures/captions in
    /// recovered reading order, with metadata, per-page geometry, and inline
    /// styling. The primary document-parser surface.
    Parse(ParseArgs),
    /// Build a typed, ordered document model — headings/paragraphs/lists/figures/
    /// captions/tables in recovered reading order — as JSON or readable markdown.
    /// Superseded by `parse` (kept as a thin alias for back-compat).
    DocumentModel(DocumentModelArgs),
    /// Extract structured key-value fields (invoice number/date/total, receipt
    /// merchant/amount, form label→value pairs, line items) to JSON. Combines
    /// exact AcroForm fields, a spatial label→value engine, and document-type
    /// profiles — works on digital-born and OCR'd documents alike.
    ExtractFields(ExtractFieldsArgs),
    /// Report AcroForm field trees, inheritance, widgets, XFA, and form diagnostics
    FormsReport(FormsReportArgs),
    /// Inventory XFA packets, ordering, XML safety, classification, and limits
    XfaReport(XfaReportArgs),
    /// Extract supported static XFA fields, datasets, layout, scripts, and provenance
    XfaExtract(XfaReportArgs),
    /// Build a PDF overlay preview through the existing page renderer/writer path
    XfaRender(XfaRenderArgs),
    /// Flatten the supported static XFA subset with an explicit preservation mode
    XfaFlatten(XfaFlattenArgs),
    /// Apply the dedicated XFA active-content sanitizer policy
    XfaSanitize(XfaSanitizeArgs),
    /// Inventory XFA scripts/events and the default sandbox policy
    XfaScriptReport(XfaReportArgs),
    /// Report XFA security, sanitizer, signature, and redaction posture
    XfaSecurityReport(XfaReportArgs),
    /// Run/report the bounded minimal dynamic XFA runtime
    XfaRuntimeReport(XfaRuntimeReportArgs),
    /// Export AcroForm field values as JSON, FDF, or XFDF
    FormsExport(FormsExportArgs),
    /// Import AcroForm field values from JSON, FDF, or XFDF
    FormsImport(FormsImportArgs),
    /// Report annotations, QuadPoints, appearances, and unsafe actions
    AnnotationsReport(AnnotationsReportArgs),
    /// Export deterministic, bounded annotation XFDF
    AnnotationXfdfExport(AnnotationXfdfExportArgs),
    /// Create/update/delete annotations from secure XFDF
    AnnotationXfdfImport(AnnotationXfdfImportArgs),
    /// Generate deterministic annotation appearance streams
    AnnotationAppearanceGenerate(AnnotationAppearanceGenerateArgs),
    /// Report annotation appearance generation decisions
    AnnotationAppearanceReport(AnnotationAppearanceReportArgs),
    /// Inventory RichMedia, Sound, Movie, Screen, Rendition, and 3D content
    RichMediaReport(AnnotationMediaRedactionReportArgs),
    /// Apply an explicit rich-media sanitizer policy
    RichMediaSanitize(RichMediaSanitizeArgs),
    /// Flatten static media posters without decoding or executing media
    RichMediaFlattenPoster(AnnotationMediaRedactionOutputArgs),
    /// Plan/apply polygonal non-axis image redaction
    RedactImageNonaxis(NonAxisRedactionArgs),
    /// Combined annotation/media redaction report
    AnnotationMediaRedactionReport(AnnotationMediaRedactionReportArgs),
    /// Secure mask/soft-mask image redaction
    RedactImageMask(NonAxisRedactionArgs),
    /// Secure inline-image partial redaction
    RedactInlineImage(NonAxisRedactionArgs),
    /// Inventory embedded and associated files
    AssociatedFilesReport(AnnotationMediaRedactionReportArgs),
    /// Extract one associated file by stable id
    AssociatedFilesExtract(AssociatedFilesExtractArgs),
    /// Add an associated file
    AssociatedFilesAdd(AssociatedFilesAddArgs),
    /// Update one owner-specific associated-file association
    AssociatedFilesUpdate(AssociatedFilesUpdateArgs),
    /// Remove associated files by stable id
    AssociatedFilesRemove(AssociatedFilesRemoveArgs),
    /// Apply an associated-file sanitizer policy
    AssociatedFilesSanitize(AssociatedFilesSanitizeArgs),
    /// Analyze signature impact for an edit operation
    EditSignatureImpact(EditPolicyArgs),
    /// Report the signature-aware edit-policy decision
    EditPolicyReport(EditPolicyArgs),
    /// Combined secure mutation report
    SecureMutationReport(AnnotationMediaRedactionReportArgs),
    /// Combined secure mutation closeout closure report
    SecureMutationCloseoutReport(AnnotationMediaRedactionReportArgs),
    /// Inventory PDF form JavaScript and action graphs without executing scripts
    FormJsReport(AnnotationMediaRedactionReportArgs),
    /// Sanitize PDF actions under an explicit form action policy policy
    FormJsSanitize(FormActionPolicySanitizeArgs),
    /// Evaluate the bounded calculation subset, write values, then remove actions
    FormJsFlattenValues(FormActionPolicySanitizeArgs),
    /// Combined interactive/data consistency close-out report
    InteractiveDataReport(AnnotationMediaRedactionReportArgs),
    /// Audit DOCX pagination structure for one layout mode
    WordPaginationAudit(WordPaginationAuditArgs),
    /// Combined form action policy form/action/interactive/DOCX report
    FormActionPolicyReport(AnnotationMediaRedactionReportArgs),
    /// Combined advanced editing vertical/RTL, same-width, vector, and ink report
    AdvancedEditingReport(AnnotationMediaRedactionReportArgs),
    /// Combined advanced editing closeout multi-run, Form ownership, and appearance report
    AdvancedEditingCloseoutReport(AnnotationMediaRedactionReportArgs),
    /// Combined writer history raster/vector, font, persistent history, and writer report
    WriterHistoryReport(AnnotationMediaRedactionReportArgs),
    /// Combined compression and Office zopfli, dedup, Office conversion, and benchmark report
    CompressionOfficeReport(AnnotationMediaRedactionReportArgs),
    /// Combined crypto writer deterministic writer, PubSec, and AES-GCM report
    CryptoWriterReport(AnnotationMediaRedactionReportArgs),
    /// Audit deterministic writer reproducibility through the production writer
    WriterDeterminismAudit(AnnotationMediaRedactionReportArgs),
    /// Produce an object-aware deterministic writer external diff report
    WriterExternalDiff(AnnotationMediaRedactionReportArgs),
    /// Report advanced writer canonicalization and close-out posture
    WriterCloseoutReport(AnnotationMediaRedactionReportArgs),
    /// Report public-key security-handler support and exact unsupported status
    PubsecReport(AnnotationMediaRedactionReportArgs),
    /// Public-key security-handler decrypt command; report-only until supported
    PubsecDecrypt(CryptoWriterCryptoReportArgs),
    /// Public-key security-handler encrypt command for supported KeyTrans recipients
    PubsecEncrypt(CryptoWriterCryptoReportArgs),
    /// Add a public-key recipient by full-rewrite re-encryption to the supplied recipient set
    PubsecAddRecipient(CryptoWriterCryptoReportArgs),
    /// Remove a public-key recipient by full-rewrite re-encryption to the supplied recipient set
    PubsecRemoveRecipient(CryptoWriterCryptoReportArgs),
    /// Replace public-key recipients by full-rewrite re-encryption to the supplied recipient set
    PubsecReplaceRecipient(CryptoWriterCryptoReportArgs),
    /// Public-key security-handler full-rewrite re-encrypt command
    PubsecReencrypt(CryptoWriterCryptoReportArgs),
    /// Public-key decrypt/edit/re-encrypt workflow using the supported full-rewrite policy
    PubsecDecryptEditReencrypt(CryptoWriterCryptoReportArgs),
    /// Report ISO/TS 32004 PDF-MAC structure and exact verification posture
    PdfMacReport(AnnotationMediaRedactionReportArgs),
    /// Verify ISO/TS 32004 PDF-MAC where supported; never claims validity from structure alone
    PdfMacVerify(AnnotationMediaRedactionReportArgs),
    /// Create AESV4 encrypted PDF output with a standalone ISO/TS 32004 PDF-MAC token
    PdfMacCreate(CryptoWriterCryptoReportArgs),
    /// Report PDF AES-GCM support and exact remaining limits
    AesGcmReport(AnnotationMediaRedactionReportArgs),
    /// AES-GCM decrypt command; writes plaintext only with --pdf-output
    AesGcmDecrypt(CryptoWriterCryptoReportArgs),
    /// AES-GCM encrypt command; writes encrypted PDF only with --pdf-output
    AesGcmEncrypt(CryptoWriterCryptoReportArgs),
    /// Run/report crypto writer crypto tamper policy checks
    CryptoTamperTest(CryptoWriterTamperArgs),
    /// Analyze bounded raster-to-vector candidates on a page
    RasterVectorReport(WriterHistoryRasterVectorArgs),
    /// Alias for raster-vector-report; exports the vector model by default
    RasterVectorize(WriterHistoryRasterVectorArgs),
    /// Inspect safe font reconstruction eligibility and glyph hook policy
    FontReconstruct(AnnotationMediaRedactionReportArgs),
    /// Inspect safe font reconstruction eligibility and glyph hook policy
    FontReconstructionReport(AnnotationMediaRedactionReportArgs),
    /// Report writer history persistent edit history structures
    HistoryReport(WriterHistoryHistoryArgs),
    /// Export a writer history persistent history snapshot report
    HistorySnapshot(WriterHistoryHistoryArgs),
    /// Validate writer history persistent history restore posture
    HistoryRestore(WriterHistoryHistoryArgs),
    /// Report writer history persistent history diff posture
    HistoryDiff(WriterHistoryHistoryArgs),
    /// Inspect object-stream packing eligibility and xref-stream results
    ObjectStreamReport(AnnotationMediaRedactionReportArgs),
    /// Save a full-rewrite PDF using object streams and an xref stream
    SaveObjectStreams(WriterHistorySaveObjectStreamsArgs),
    /// Save a full-rewrite PDF with compression and Office compression and dedup planning
    CompressionOfficeOptimize(CompressionOfficeOptimizeArgs),
    /// Inspect a DOCX/PPTX/XLSX package under compression and Office security limits
    CompressionOfficeOfficeInspect(CompressionOfficeOfficeArgs),
    /// Convert DOCX/PPTX/XLSX to PDF through the compression and Office native conversion report path
    CompressionOfficeOfficeToPdf(CompressionOfficeOfficeToPdfArgs),
    /// Analyze or edit a logical text range spanning text-showing operators
    EditTextRange(AdvancedEditingCloseoutTextRangeArgs),
    /// Report source-level provenance for an operator-preserving text selection
    ProvenanceReport(SourceEditingTextSelectionArgs),
    /// Check whether an operator-preserving edit can be applied without escalation
    EditEligibility(SourceEditingTextSelectionArgs),
    /// Replace text by mutating the original text-showing operator, not an overlay
    EditTextOperator(SourceEditingTextEditArgs),
    /// Edit one source path/vector operator through the source editing routed API
    EditPathOperator(SourceEditingPathEditArgs),
    /// Report exact unsupported/source eligibility for image occurrence edits
    EditImageOccurrence(SourceEditingImageArgs),
    /// Clone/edit one Form occurrence through source-level vector routing
    EditFormOccurrence(SourceEditingPathEditArgs),
    /// Report the source editing true-editing operation schema and limits
    EditOperationReport(AnnotationMediaRedactionReportArgs),
    /// editing transactions editable scene/snapshot/transaction/font architecture report
    EditingTransactionsReport(AnnotationMediaRedactionReportArgs),
    /// Build a source-linked editable scene graph
    SceneReport(EditingTransactionsSceneReportArgs),
    /// Resolve a scene node by id, point, or region
    SceneSelect(EditingTransactionsSceneSelectArgs),
    /// Plan an atomic editing transactions edit transaction
    TransactionPlan(EditingTransactionsTransactionArgs),
    /// Apply an atomic editing transactions edit transaction
    TransactionApply(EditingTransactionsTransactionArgs),
    /// Report editing transactions exact undo/restoration policy for a transaction
    TransactionUndo(EditingTransactionsTransactionArgs),
    /// Report PDF-code/CID/GID/Unicode/grapheme/shaping mapping
    TextMap(EditingTransactionsTextArgs),
    /// Preview canonical OpenType shaping for text
    ShapeText(EditingTransactionsTextArgs),
    /// Plan deterministic font subset reconstruction
    FontSubsetPlan(EditingTransactionsTextArgs),
    /// Alias for font-subset-plan; editing transactions reports exact build limits
    FontSubsetBuild(EditingTransactionsTextArgs),
    /// Report deterministic font substitution policy and scoring
    FontSubstitutionReport(EditingTransactionsFontSubstitutionArgs),
    /// Scene-facing local text edit compiled to source-level operator mutation
    SceneEditText(EditingTransactionsTransactionArgs),
    /// text reflow geometric/semantic reflow architecture report
    TextReflowReport(AnnotationMediaRedactionReportArgs),
    /// Analyze source-linked geometric and semantic layout
    LayoutAnalyze(TextReflowReflowArgs),
    /// Report deterministic reading order
    ReadingOrderReport(AnnotationMediaRedactionReportArgs),
    /// Report cross-column/cross-page flow graph
    FlowGraphReport(AnnotationMediaRedactionReportArgs),
    /// Preview text reflow reflow without mutating
    ReflowPreview(TextReflowReflowArgs),
    /// Report ordered text reflow overflow evidence without mutating
    OverflowReport(TextReflowReflowArgs),
    /// Report bounded text reflow hard/soft constraints without mutating
    ReflowConstraints(TextReflowReflowArgs),
    /// Report text reflow confidence/review enforcement without mutating
    ReflowConfidence(TextReflowReflowArgs),
    /// Validate a completed local text reflow reflow with explicit source, output, and request files
    ReflowValidate(TextReflowValidateArgs),
    /// Apply supported GeometricBlock reflow
    ReflowRegion(TextReflowReflowArgs),
    /// Apply supported SemanticDocument reflow
    ReflowDocument(TextReflowReflowArgs),
    /// Replay and execute a verified text reflow undo without overwriting either input PDF
    ReflowUndo(TextReflowUndoArgs),
    /// Store/preview a reviewed semantic-structure correction
    ReflowApproveStructure(TextReflowStructureCorrectionArgs),
    /// Report text reflow transaction and undo evidence
    ReflowOperationReport(TextReflowReflowArgs),
    /// Report document subsystems tables, math, OCR, annotation, form, and XFA capabilities
    DocumentSubsystemsReport(AnnotationMediaRedactionReportArgs),
    /// Analyze source-linked document subsystems tables, math, OCR layers, annotations, forms, and XFA
    DocumentSubsystemsAnalyze(AnnotationMediaRedactionReportArgs),
    /// Plan a typed document subsystems operation from an explicit JSON request
    DocumentSubsystemsPlan(DocumentSubsystemsRequestArgs),
    /// Apply a typed document subsystems operation and write a distinct output PDF
    DocumentSubsystemsApply(DocumentSubsystemsApplyArgs),
    /// Replay and undo a document subsystems operation into a distinct restored PDF
    DocumentSubsystemsUndo(DocumentSubsystemsUndoArgs),
    /// Report document security accessibility, redaction, sanitizer, and residual-verification capabilities
    DocumentSecurityReport(AnnotationMediaRedactionReportArgs),
    /// Analyze document security tagged-PDF, accessibility, redaction, and sanitizer state
    DocumentSecurityAnalyze(AnnotationMediaRedactionReportArgs),
    /// Plan a typed document security operation from an explicit JSON request
    DocumentSecurityPlan(DocumentSecurityRequestArgs),
    /// Apply a typed document security operation and write a distinct output PDF
    DocumentSecurityApply(DocumentSecurityApplyArgs),
    /// Replay and undo a document security operation into a distinct restored PDF
    DocumentSecurityUndo(DocumentSecurityUndoArgs),
    /// Run document security residual verification from a JSON term list
    DocumentSecurityVerifyResidual(DocumentSecurityVerifyArgs),
    /// Alias for vector-list focused on Form invocation ownership
    FormInstanceReport(AdvancedEditingVectorListArgs),
    /// Alias for vector-edit with clone-edit-one-instance policy
    FormCloneOne(AdvancedEditingVectorEditArgs),
    /// Alias for vector-list focused on shared annotation appearances
    AnnotationAppearanceSharedReport(AdvancedEditingVectorListArgs),
    /// Alias for vector-edit with clone-edit-one-instance appearance policy
    AnnotationAppearanceCloneOne(AdvancedEditingVectorEditArgs),
    /// List stable editable vector objects on a page
    VectorList(AdvancedEditingVectorListArgs),
    /// Edit one stable vector object using a VectorEditOperation JSON file
    VectorEdit(AdvancedEditingVectorEditArgs),
    /// Delete one stable vector object
    VectorDelete(AdvancedEditingVectorDirectArgs),
    /// Duplicate one stable vector object
    VectorDuplicate(AdvancedEditingVectorDirectArgs),
    /// Fit one Ink annotation and regenerate its cubic appearance
    InkFit(AdvancedEditingInkFitArgs),
    /// Incrementally edit a form value under signature policy
    EditForm(EditFormArgs),
    /// Incrementally add/update an annotation under signature policy
    EditAnnotation(EditMutationArgs),
    /// Incrementally edit page rotation or CropBox under signature policy
    EditPageProperty(EditMutationArgs),
    /// Flatten common page annotations into page content
    AnnotationsFlatten(AnnotationsFlattenArgs),
    /// Report page boxes, labels/outlines/destinations, and page-op preservation risks
    PagesReport(PagesReportArgs),
    /// roadmap closure 07 interactive/data-layer report
    InteractiveReport(InteractiveReportArgs),
    /// Apply true redaction from search terms and/or explicit rectangles
    Redact(RedactArgs),
    /// Split a PDF into RAG-ready semantic chunks (structure-aware, token-sized,
    /// with overlap + heading context) as a JSON chunks array for embedding
    /// pipelines. Tables/figures stay intact; headings drive boundaries.
    Chunk(ChunkArgs),
    /// Export the Semantic Closeout semantic binding bundle, advanced RAG chunks,
    /// dictionary tokens, tables, search results, or ML proposal status as JSON.
    SemanticExport(SemanticExportArgs),
    /// Score an extraction result against ground truth using standard metrics
    /// (CER/WER/reading-order/table cell-F1/TEDS/field-F1/block-type accuracy).
    /// Reads a ScoreInput JSON (file or stdin), writes a ScoreOutput JSON. The
    /// pure-Rust scoring core the extraction benchmark harness drives.
    EvalScore(EvalScoreArgs),
    /// Extract embedded images from a PDF as a ZIP
    ExtractImages(ExtractImagesArgs),
    /// Render PDF pages to images as a ZIP
    Render(RenderArgs),
    /// Emit Native Renderer display-list/native-replay counters for rendered pages
    RenderCompare(RenderCompareArgs),
    /// Rasterize PDF pages to individual JPG/PNG image files
    #[command(alias = "pdf-to-image")]
    PdfToJpg(PdfToJpgArgs),
    /// Wrap one or more JPG/PNG files into a new PDF
    ImageToPdf(ImageToPdfArgs),
    /// Convert table-oriented PDF content to an XLSX workbook
    PdfToXlsx(PdfToXlsxArgs),
    /// Convert PDF pages to editable PPTX slides
    PdfToPptx(PdfToPptxArgs),
    /// Convert PDF content to a flowing DOCX document
    PdfToDocx(PdfToDocxArgs),
    /// Convert PDF content to semantic HTML through the editable model
    PdfToHtml(PdfToHtmlArgs),
    /// Convert PDF content to Markdown through the editable model
    PdfToMarkdown(PdfToMarkdownArgs),
    /// Convert PDF content to JSON through the editable model
    PdfToJson(PdfToJsonArgs),
    /// Export the shared Advanced Rendering editable document model as JSON
    ExportEditableModel(ExportEditableModelArgs),
    /// Replace text by full-rewrite redaction plus replacement text overlay
    EditText(EditTextArgs),
    /// Append a small incremental text overlay update for writer verification
    SaveIncremental(SaveIncrementalArgs),
    /// Convert a DOCX document to PDF with Wellfriend's native writer
    DocxToPdf(OfficeToPdfArgs),
    /// Convert an XLSX workbook to PDF with Wellfriend's native writer
    XlsxToPdf(OfficeToPdfArgs),
    /// Convert a PPTX presentation to PDF with Wellfriend's native writer
    PptxToPdf(OfficeToPdfArgs),
    /// Analyze whether a PDF has a real text layer
    Analyze(AnalyzeArgs),
    /// Merge several PDFs into one (pdfunite-equivalent)
    Merge(MergeArgs),
    /// Split a PDF into separate single-page PDFs (pdfseparate-equivalent)
    Split(SplitArgs),
    /// Extract a subset of pages into a new PDF
    ExtractPages(ExtractPagesArgs),
    /// Report document metadata and structural facts (pdfinfo-equivalent)
    Info(InfoArgs),
    /// Emit shared SDK feature, codec, scheduler, corpus, and fuzz capability report
    FeatureReport(FeatureReportArgs),
    /// Emit structured parser diagnostics, repair/audit status, and source metrics
    ParserReport(ParserReportArgs),
    /// Exercise codec subprocess isolation policy and emit a JSON diagnostic report
    CodecIsolationReport(CodecIsolationReportArgs),
    /// List the fonts used in a PDF (pdffonts-equivalent)
    Fonts(FontsArgs),
    /// List or extract embedded file attachments (pdfdetach-equivalent)
    Detach(DetachArgs),
    /// Convert a PDF to HTML or XML (pdftohtml-equivalent)
    ToHtml(ToHtmlArgs),
    /// Verify digital signatures in a PDF (pdfsig-equivalent)
    VerifySig(VerifySigArgs),
    /// List PDF signature fields and Signature Validation validation state
    SignatureList(VerifySigArgs),
    /// Validate PDF detached signatures with Signature Validation policy inputs
    SignatureVerify(VerifySigArgs),
    /// Validate PAdES baseline posture with Signature Validation policy inputs
    PadesVerify(VerifySigArgs),
    /// Plan a Pades LTV signature-preserving incremental form fill
    SignaturePreservingPlan(SignaturePreservingFormArgs),
    /// Apply a Pades LTV signature-preserving incremental form fill
    SignaturePreservingEdit(SignaturePreservingFormArgs),
    /// Validate a caller-supplied RFC 3161 signature timestamp token
    TimestampVerify(TimestampVerifyArgs),
    /// List validated signature timestamp tokens for PDF signatures
    SignatureTimestamps(VerifySigArgs),
    /// Inspect DSS/VRI evidence and replay posture for PDF signatures
    DssInspect(VerifySigArgs),
    /// Verify Pades LTV PAdES LTV status from timestamp and DSS/VRI evidence
    LtvVerify(VerifySigArgs),
    /// Report achieved PAdES baseline/timestamp/LT level for PDF signatures
    PadesLevelReport(VerifySigArgs),
    /// Build signer certificate paths for PDF signatures with Signature Validation inputs
    CertificatePathBuild(VerifySigArgs),
    /// Validate signer certificate paths against explicit Signature Validation trust anchors
    CertificatePathVerify(VerifySigArgs),
    /// Validate OCSP evidence for PDF signature certificate paths
    OcspCheck(VerifySigArgs),
    /// Validate CRL evidence for PDF signature certificate paths
    CrlCheck(VerifySigArgs),
    /// Evaluate supplied revocation evidence for PDF signatures
    RevocationCheck(VerifySigArgs),
    /// Fetch bounded AIA/OCSP/CRL evidence and export a replay bundle
    EvidenceFetch(VerifySigArgs),
    /// Validate signatures and export only accepted evidence for later replay
    EvidenceExport(VerifySigArgs),
    /// Verify a signature using an imported evidence bundle with network disabled
    EvidenceVerify(VerifySigArgs),
    /// Replay an exported evidence bundle with network retrieval disabled
    EvidenceReplay(VerifySigArgs),
    /// Emit encryption, signature, and active-content security diagnostics
    SecurityReport(SecurityReportArgs),
    /// Alias for verify-sig with Annotation Ocg Rendering signature status fields
    SignatureReport(VerifySigArgs),
    /// Remove active/risky PDF content according to a sanitizer policy
    Sanitize(SanitizeArgs),
    /// Validate supported PDF/A, PDF/UA, PDF/X, and security profile subsets
    Validate(ValidateArgs),
    /// Incremental Signing Standards clause-mapped PDF/A validation (ISO 19005)
    PdfaValidate(StandardsValidateArgs),
    /// Incremental Signing Standards clause-mapped PDF/UA validation (ISO 14289-1)
    PdfuaValidate(StandardsValidateArgs),
    /// Incremental Signing Standards clause-mapped PDF/X validation (ISO 15930)
    PdfxValidate(StandardsValidateArgs),
    /// Incremental Signing Standards combined PDF/A+UA+X clause-mapped validation with cross-profile conflicts
    StandardsValidate(StandardsValidateArgs),
    /// Incremental Signing Standards append-only incremental signing (approval or certification)
    SignatureSign(SignatureSignArgs),
    /// Incremental Signing Standards signature /Contents placeholder capacity plan (writes no output)
    SignaturePlanPlaceholder(SignatureSignArgs),
    /// Write a deterministic canonical full-rewrite copy and audit report
    Canonicalize(CanonicalizeArgs),
    /// Encrypt a PDF with a password (RC4-128 / AES-128 / AES-256). AES-256 default.
    Encrypt(EncryptArgs),
    /// Write an unencrypted normalized copy of a password-opened PDF
    #[command(alias = "unlock")]
    Decrypt(DecryptArgs),
    /// Add a text or image watermark to a PDF
    Watermark(WatermarkArgs),
    /// Add page numbers to a PDF
    AddPageNumbers(PageNumbersArgs),
    /// Reorder, delete, duplicate, or insert pages in a PDF
    Organize(OrganizeArgs),
    /// Set page rotation (/Rotate) and write a new PDF (absolute or relative)
    Rotate(RotateArgs),
    /// Set page CropBox values while preserving the source graph
    PagesCrop(PagesCropArgs),
    /// Scale pages visually into a new PDF
    PagesScale(PagesScaleArgs),
    /// Create a visual n-up imposed PDF
    PagesNup(PagesNupArgs),
    /// Shrink a PDF (garbage-collect + recompress) without changing content
    Optimize(OptimizeArgs),
    /// Write a clean, normalized copy of a damaged PDF (qpdf --check passes)
    Repair(RepairArgs),
    /// Produce a linearized (fast-web-view) PDF
    Linearize(LinearizeArgs),
}

#[derive(Parser)]
struct EncryptArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// Output file
    #[arg(short, long, default_value = "encrypted.pdf")]
    output: PathBuf,
    /// User (open) password
    #[arg(long, default_value = "")]
    user_pw: String,
    /// Owner (full-permissions) password; defaults to the user password
    #[arg(long, default_value = "")]
    owner_pw: String,
    /// Algorithm: aes256 (default), aesgcm, aes128, or rc4
    #[arg(long, default_value = "aes256")]
    algo: String,
    /// Permission bitmask (/P), signed 32-bit; default -1 grants everything
    #[arg(long, default_value = "-1", allow_negative_numbers = true)]
    permissions: i32,
    /// Password to open the input, if it is already encrypted
    #[arg(long)]
    password: Option<String>,
    /// Emit a JSON result summary instead of a human line
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct RotateArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// Output file
    #[arg(short, long, default_value = "rotated.pdf")]
    output: PathBuf,
    /// Rotation angle in degrees (0/90/180/270, normalized)
    #[arg(short, long)]
    angle: i32,
    /// Apply the angle RELATIVE to each page's current rotation (default: absolute)
    #[arg(long)]
    relative: bool,
    /// Page range: all (default), 1, 2-5, or 1,3,7
    #[arg(short, long, default_value = "all")]
    pages: String,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
    /// Emit a JSON result summary
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct OptimizeArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// Output file
    #[arg(short, long, default_value = "optimized.pdf")]
    output: PathBuf,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
    /// Emit a JSON result summary (sizes + streams recompressed)
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct RepairArgs {
    /// Path to the (possibly damaged) input PDF
    pdf: PathBuf,
    /// Output file
    #[arg(short, long, default_value = "repaired.pdf")]
    output: PathBuf,
    /// Password if the input is encrypted (repaired copy is written unencrypted)
    #[arg(long)]
    password: Option<String>,
    /// Emit a JSON result summary
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct LinearizeArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// Output file
    #[arg(short, long, default_value = "linearized.pdf")]
    output: PathBuf,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
    /// Emit a JSON result summary
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct CodecIsolationReportArgs {
    /// PDF stream filter to decode, for example FlateDecode or RunLengthDecode
    #[arg(long, default_value = "FlateDecode")]
    filter: String,
    /// Read encoded stream bytes from this file
    #[arg(long, conflicts_with_all = ["input_hex", "sample_text"])]
    input_file: Option<PathBuf>,
    /// Encoded stream bytes as hex, for small reproducible examples
    #[arg(long, conflicts_with_all = ["input_file", "sample_text"])]
    input_hex: Option<String>,
    /// Convenience sample text. For FlateDecode this is zlib-compressed first.
    #[arg(long, conflicts_with_all = ["input_file", "input_hex"])]
    sample_text: Option<String>,
    /// Isolation policy: in_process, isolated_preferred, isolated_required, report_only, disabled
    #[arg(long, default_value = "in_process")]
    policy: String,
    /// Explicit worker binary path. Otherwise WELLFRIENDPDF_CODEC_WORKER or a sibling binary is used.
    #[arg(long)]
    worker: Option<PathBuf>,
    /// Worker timeout in milliseconds
    #[arg(long, default_value_t = 2000)]
    timeout_ms: u64,
    /// Maximum decoded bytes accepted from the worker or in-process fallback
    #[arg(long, default_value_t = 536870912)]
    max_output_bytes: u64,
}

#[derive(Parser)]
struct DecryptArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// Output file
    #[arg(short, long, alias = "out", default_value = "decrypted.pdf")]
    output: PathBuf,
    /// Password to open the encrypted input
    #[arg(long)]
    password: Option<String>,
    /// Emit a JSON result summary
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct WatermarkArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// Output file
    #[arg(short, long, alias = "out", default_value = "watermarked.pdf")]
    output: PathBuf,
    /// Text watermark
    #[arg(long)]
    text: Option<String>,
    /// Image watermark (JPG/PNG)
    #[arg(long)]
    image: Option<PathBuf>,
    /// Page range: all, 1, 2-5, or 1,3,7
    #[arg(short, long, default_value = "all")]
    pages: String,
    /// Position: center, tile, top-left, top-center, top-right, bottom-left,
    /// bottom-center, or bottom-right
    #[arg(long, default_value = "center")]
    position: String,
    /// Opacity in 0..1
    #[arg(long, default_value = "0.28")]
    opacity: f64,
    /// Text rotation angle in degrees
    #[arg(long, default_value = "45")]
    rotation: f64,
    /// Text font size in points
    #[arg(long, default_value = "64")]
    font_size: f64,
    /// Text color as #RRGGBB
    #[arg(long, default_value = "#8c8c8c")]
    color: String,
    /// Image scale relative to page box
    #[arg(long, default_value = "0.5")]
    scale: f64,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
    /// Emit a JSON result summary
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct PageNumbersArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// Output file
    #[arg(short, long, alias = "out", default_value = "numbered.pdf")]
    output: PathBuf,
    /// Page range: all, 1, 2-5, or 1,3,7
    #[arg(short, long, default_value = "all")]
    pages: String,
    /// Position: top-left, top-center, top-right, bottom-left, bottom-center,
    /// or bottom-right
    #[arg(long, default_value = "bottom-center")]
    position: String,
    /// Format string using {n}, {page}, and/or {total}
    #[arg(long, default_value = "Page {n} of {total}")]
    format: String,
    /// Starting number for the first physical page
    #[arg(long, default_value = "1")]
    start: isize,
    /// Font size in points
    #[arg(long, default_value = "10")]
    font_size: f64,
    /// Text color as #RRGGBB
    #[arg(long, default_value = "#333333")]
    color: String,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
    /// Emit a JSON result summary
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct OrganizeArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// Output page order, e.g. 1,2,5,3,4,3. Repeats duplicate pages; omissions
    /// delete pages.
    #[arg(long, default_value = "all")]
    order: String,
    /// Optional second PDF to insert
    #[arg(long)]
    insert_from: Option<PathBuf>,
    /// Pages from --insert-from to insert
    #[arg(long, default_value = "all")]
    insert_pages: String,
    /// 1-based output position before which inserted pages are placed
    #[arg(long, default_value = "1")]
    insert_at: usize,
    /// Output file
    #[arg(short, long, alias = "out", default_value = "organized.pdf")]
    output: PathBuf,
    /// Password for the primary PDF
    #[arg(long)]
    password: Option<String>,
    /// Password for --insert-from
    #[arg(long)]
    insert_password: Option<String>,
    /// Emit a JSON result summary
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct ExtractTextArgs {
    /// Path to the PDF file
    pdf: PathBuf,
    /// Output file, defaults to stdout
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Page range: all, 1, 2-5, or 1,3,7
    #[arg(short, long, default_value = "all")]
    pages: String,
    /// Include page numbers in output
    #[arg(long)]
    page_numbers: bool,
    /// Layout-aware extraction: recover reading order across columns/blocks via
    /// geometric XY-cut segmentation (correct multi-column order, unlike a plain
    /// top-to-bottom dump). Additive; the default extraction is unchanged.
    #[arg(long)]
    structured: bool,
    /// Semantic extraction: use tagged-PDF structure when present, falling back
    /// to geometric layout analysis when absent.
    #[arg(long)]
    semantic: bool,
    /// Output format for --structured/--semantic: text, json, or model-json
    /// (Native Renderer geometry/provenance model). Ignored without either flag.
    #[arg(long, default_value = "text")]
    format: String,
    /// Include detailed structure attachment in model-json output.
    #[arg(long)]
    include_structure: bool,
    /// Include detailed char/span provenance in model-json output.
    #[arg(long)]
    include_provenance: bool,
    /// CJK tokenization for model-json: char, simple, or dictionary. Dictionary
    /// mode uses the provider-backed built-in fixture by default and preserves
    /// raw extracted text.
    #[arg(long, default_value = "char")]
    cjk_segmentation: String,
    /// Include invisible/hidden text in model-json output.
    #[arg(long)]
    include_hidden: bool,
    /// Restrict extraction to a page box in PDF user-space points:
    /// x0,y0,x1,y1 with origin bottom-left.
    #[arg(long)]
    region: Option<String>,
    /// Extraction profile: fast-text, layout-faithful, tables-focused, or rag-chunks.
    #[arg(long, default_value = "fast-text")]
    profile: String,
    /// OCR scanned (image-only) pages with Tesseract and extract the recovered
    /// text, instead of returning nothing for pages with no text layer. Accepts
    /// a policy: `--ocr`/`--ocr auto` OCRs classifier-detected scanned pages,
    /// `--ocr force` re-OCRs every selected page, `--ocr off` disables it.
    /// Routes through the OCR-aware document parser. Requires the `tesseract`
    /// binary on PATH and a CLI built with `--features ocr`. Mutually exclusive
    /// with --structured/--semantic.
    #[arg(long, num_args = 0..=1, default_missing_value = "auto")]
    ocr: Option<String>,
    /// OCR languages (Tesseract codes), comma- or plus-separated, e.g. `eng` or
    /// `eng,deu`. The matching tessdata packs must be installed.
    #[arg(long, default_value = "eng")]
    ocr_lang: String,
    /// DPI at which scanned pages are rasterized for OCR (~300 is the sweet spot).
    #[arg(long, default_value = "300")]
    ocr_dpi: u32,
    /// Password for encrypted PDFs (the empty user password is tried automatically)
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct ExtractTablesArgs {
    /// Path to the PDF file
    pdf: PathBuf,
    /// Output file, defaults to stdout
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Page range: all, 1, 2-5, or 1,3,7
    #[arg(short, long, default_value = "all")]
    pages: String,
    /// Output format: csv (flattened), json (structured), or html (span/header table)
    #[arg(short, long, default_value = "csv")]
    format: String,
    /// Emit the span/header/nested structure model. JSON always includes the
    /// structured fields; this flag is accepted for explicit CLI workflows.
    #[arg(long)]
    structure: bool,
    /// Minimum detection confidence to include a table (0.0-1.0). Borderless
    /// tables carry lower confidence; raise this to keep only high-confidence
    /// (typically ruled) tables.
    #[arg(long, default_value = "0.0")]
    min_confidence: f64,
    /// Restrict extraction to a page box in PDF user-space points:
    /// x0,y0,x1,y1 with origin bottom-left.
    #[arg(long)]
    region: Option<String>,
    /// Accepted for surface consistency with the other extract commands, but
    /// table-grid reconstruction from OCR'd word boxes is not yet supported (a
    /// known gap — see docs/parser_benchmark.md). Passing --ocr errors with a
    /// pointer to `extract-fields --ocr` / `extract-text --ocr`, which DO OCR.
    #[arg(long, num_args = 0..=1, default_missing_value = "auto")]
    ocr: Option<String>,
    /// OCR languages (unused; --ocr is not supported for table extraction).
    #[arg(long, default_value = "eng")]
    ocr_lang: String,
    /// OCR DPI (unused; --ocr is not supported for table extraction).
    #[arg(long, default_value = "300")]
    ocr_dpi: u32,
    /// Password for encrypted PDFs (the empty user password is tried automatically)
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct ParseArgs {
    /// Path to the PDF file
    pdf: PathBuf,
    /// Output file, defaults to stdout
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Page range: all, 1, 2-5, or 1,3,7
    #[arg(short, long, default_value = "all")]
    pages: String,
    /// Output format: markdown (RAG/AI-facing), json (full faithful model), or
    /// html (semantic, for viewing)
    #[arg(short, long, default_value = "markdown")]
    format: String,
    /// Keep page furniture (running headers/footers, page numbers) in the body
    /// and output. By default furniture is omitted (it is usually noise for RAG);
    /// it is always retained in the JSON per-page view regardless.
    #[arg(long)]
    keep_furniture: bool,
    /// Emit page-boundary markers in Markdown/HTML output (a comment + rule, or
    /// an <hr>) so downstream can attribute content to pages.
    #[arg(long)]
    mark_page_breaks: bool,
    /// Annotate each block with its source page + bounding box (HTML data
    /// attributes / Markdown trailing comments) for RAG citation/traceability.
    #[arg(long)]
    provenance: bool,
    /// Write extracted figure images into this directory and reference them by
    /// path (reserved; image bytes are surfaced in a later stage).
    #[arg(long)]
    images_dir: Option<PathBuf>,
    /// De-hyphenate words split across line ends (compi-\nlation → compilation).
    /// RAG-friendly; off by default to preserve extracted characters verbatim.
    #[arg(long)]
    dehyphenate: bool,
    /// Normalize ligature codepoints to plain letters (ﬁ→fi). Off by default.
    #[arg(long)]
    normalize_ligatures: bool,
    /// Drop blocks below this classification confidence (0.0-1.0)
    #[arg(long, default_value = "0.0")]
    min_confidence: f64,
    /// Extraction profile: fast-text, layout-faithful, tables-focused, or rag-chunks.
    #[arg(long, default_value = "fast-text")]
    profile: String,
    /// Detect headings in Markdown output. Use `--detect-headings=false` for
    /// flat text-like Markdown.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    detect_headings: bool,
    /// OCR scanned (image-only) pages with Tesseract instead of emitting a
    /// placeholder. Accepts a policy: `--ocr` or `--ocr auto` OCRs only
    /// classifier-detected scanned pages; `--ocr force` re-OCRs every selected
    /// page (ignoring the classifier); `--ocr off` disables OCR (the default when
    /// the flag is absent). Requires the `tesseract` binary on PATH and a CLI
    /// built with the `ocr` feature. Recovered text flows through the same
    /// layout/heading/table pipeline as digital-born text.
    #[arg(long, num_args = 0..=1, default_missing_value = "auto")]
    ocr: Option<String>,
    /// OCR languages (Tesseract codes), comma- or plus-separated, e.g.
    /// `eng` or `eng,deu`. The matching tessdata packs must be installed.
    #[arg(long, default_value = "eng")]
    ocr_lang: String,
    /// DPI at which scanned pages are rasterized for OCR (~300 is the sweet spot).
    #[arg(long, default_value = "300")]
    ocr_dpi: u32,
    /// Password for encrypted PDFs (the empty user password is tried automatically)
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct DocumentModelArgs {
    /// Path to the PDF file
    pdf: PathBuf,
    /// Output file, defaults to stdout
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Page range: all, 1, 2-5, or 1,3,7
    #[arg(short, long, default_value = "all")]
    pages: String,
    /// Output format: json (structured model) or md/markdown/text (readable)
    #[arg(short, long, default_value = "json")]
    format: String,
    /// Drop blocks below this classification confidence (0.0-1.0)
    #[arg(long, default_value = "0.0")]
    min_confidence: f64,
    /// Password for encrypted PDFs (the empty user password is tried automatically)
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct ExtractFieldsArgs {
    /// Path to the PDF file
    pdf: PathBuf,
    /// Output file, defaults to stdout
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Page range: all, 1, 2-5, or 1,3,7
    #[arg(short, long, default_value = "all")]
    pages: String,
    /// Document type guiding the field profile: auto (detect), invoice, receipt,
    /// form, or generic.
    #[arg(long, default_value = "auto")]
    r#type: String,
    /// Output format (currently json only).
    #[arg(short, long, default_value = "json")]
    format: String,
    /// Drop fields below this confidence (0.0-1.0).
    #[arg(long, default_value = "0.0")]
    min_confidence: f32,
    /// OCR scanned pages first (Tesseract). Requires the `ocr` feature + the
    /// `tesseract` binary; lets field extraction work on scanned documents.
    /// Accepts `off`/`auto`/`force`; bare `--ocr` means `auto`.
    #[arg(long, num_args = 0..=1, default_missing_value = "auto")]
    ocr: Option<String>,
    /// OCR languages (Tesseract codes), comma/plus-separated.
    #[arg(long, default_value = "eng")]
    ocr_lang: String,
    /// DPI for OCR rasterization.
    #[arg(long, default_value = "300")]
    ocr_dpi: u32,
    /// Password for encrypted PDFs (the empty user password is tried automatically)
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct FormsReportArgs {
    /// Path to the PDF file
    pdf: PathBuf,
    /// Output file, defaults to stdout
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct XfaReportArgs {
    /// Path to the PDF file
    pdf: PathBuf,
    /// Output JSON report file; defaults to stdout
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct XfaRuntimeReportArgs {
    /// Path to the PDF file
    pdf: PathBuf,
    /// Output JSON report file; defaults to stdout
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Script policy: disabled or formcalc-safe-subset
    #[arg(long, default_value = "disabled")]
    script_policy: String,
    /// Execute the supported calculate/validate event subset
    #[arg(long)]
    execute_events: bool,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct XfaRenderArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// Output PDF containing the XFA preview overlays
    #[arg(short, long, default_value = "xfa-preview.pdf")]
    output: PathBuf,
    /// Optional JSON report output; defaults to stderr summary
    #[arg(long)]
    report: Option<PathBuf>,
    /// DPI used for reopen/render hash verification
    #[arg(long, default_value = "72")]
    dpi: u32,
    /// Script policy: disabled or formcalc-safe-subset
    #[arg(long, default_value = "disabled")]
    script_policy: String,
    /// Execute the supported calculate/validate event subset
    #[arg(long)]
    execute_events: bool,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
    /// Print the JSON report to stdout
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct XfaFlattenArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// Output PDF
    #[arg(short, long, default_value = "xfa-flattened.pdf")]
    output: PathBuf,
    /// Mode: extract_only, render_preview, flatten_supported_static,
    /// flatten_and_remove_xfa, preserve_unsupported_xfa_report_only, or
    /// fail_on_unsupported
    #[arg(long, default_value = "flatten_supported_static")]
    mode: String,
    /// Optional JSON report output
    #[arg(long)]
    report: Option<PathBuf>,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
    /// Print the JSON report to stdout
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct XfaSanitizeArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// Output sanitized PDF
    #[arg(short, long, default_value = "xfa-sanitized.pdf")]
    output: PathBuf,
    /// Mode: remove_all_xfa, remove_scripts_events_connections,
    /// preserve_static_data, or flatten_then_remove
    #[arg(long, default_value = "remove_scripts_events_connections")]
    mode: String,
    /// Optional JSON report output
    #[arg(long)]
    report: Option<PathBuf>,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
    /// Print the JSON report to stdout
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct AnnotationMediaRedactionReportArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// Output JSON report; defaults to stdout
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct CryptoWriterCryptoReportArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// Output JSON report; defaults to stdout
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Output PDF path for supported AES-GCM encrypt/decrypt operations.
    #[arg(long, alias = "out")]
    pdf_output: Option<PathBuf>,
    /// Certificate path for PubSec provider input; may be repeated for output recipients.
    #[arg(long)]
    certificate: Vec<PathBuf>,
    /// Private-key path for PubSec provider input.
    #[arg(long)]
    private_key: Vec<PathBuf>,
    /// PKCS #12 / PFX provider bundle for PubSec open/decrypt.
    #[arg(long = "pfx")]
    pfx: Option<PathBuf>,
    /// File containing the PKCS #8 / PFX password bytes. Prefer this over command-line secrets.
    #[arg(long = "private-key-password-file")]
    private_key_password_file: Option<PathBuf>,
    /// Output recipient certificate path for PubSec encrypt/re-encrypt; repeat for multiple recipients.
    #[arg(long = "recipient-certificate")]
    recipient_certificate: Vec<PathBuf>,
    /// Password for already-supported Standard handler PDFs. Do not pass private-key passwords here.
    #[arg(long)]
    password: Option<String>,
    /// AES-GCM output user password for aes-gcm-encrypt. Defaults to --password or empty.
    #[arg(long)]
    user_pw: Option<String>,
    /// AES-GCM output owner password for aes-gcm-encrypt. Defaults to the user password.
    #[arg(long)]
    owner_pw: Option<String>,
    /// AES-GCM output permission bitmask (/P), signed 32-bit; default -1 grants everything.
    #[arg(long, default_value = "-1", allow_negative_numbers = true)]
    permissions: i32,
    /// Return a non-zero exit status after writing the unsupported report.
    #[arg(long)]
    fail_on_unsupported: bool,
}

#[derive(Parser)]
struct CryptoWriterTamperArgs {
    /// Output JSON report; defaults to stdout
    #[arg(short, long)]
    output: Option<PathBuf>,
}

#[derive(Parser)]
struct AnnotationMediaRedactionOutputArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// Output PDF
    #[arg(short, long, default_value = "annotation_media_redaction-output.pdf")]
    output: PathBuf,
    /// Optional JSON report path
    #[arg(long)]
    report: Option<PathBuf>,
    /// Print JSON report to stdout
    #[arg(long)]
    json: bool,
    /// Validate and report without writing the output PDF
    #[arg(long)]
    dry_run: bool,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct WriterHistoryRasterVectorArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// One-based page number
    #[arg(short, long, default_value_t = 1)]
    page: usize,
    /// Optional RasterVectorizationOptions JSON file
    #[arg(long)]
    options: Option<PathBuf>,
    /// Output JSON report; defaults to stdout
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct WriterHistoryHistoryArgs {
    /// Output JSON report; defaults to stdout
    #[arg(short, long)]
    output: Option<PathBuf>,
}

#[derive(Parser)]
struct WriterHistorySaveObjectStreamsArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// Output PDF
    #[arg(short, long, default_value = "object-stream-packed.pdf")]
    output: PathBuf,
    /// Optional JSON report path
    #[arg(long)]
    report: Option<PathBuf>,
    /// Print JSON report to stdout
    #[arg(long)]
    json: bool,
    /// Validate and report without writing the output PDF
    #[arg(long)]
    dry_run: bool,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct CompressionOfficeOptimizeArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// Output PDF
    #[arg(short, long, default_value = "compression_office-optimized.pdf")]
    output: PathBuf,
    /// Optional CompressionOfficeOptimizeOptions JSON file
    #[arg(long)]
    options: Option<PathBuf>,
    /// Optional JSON report path
    #[arg(long)]
    report: Option<PathBuf>,
    /// Print JSON report to stdout
    #[arg(long)]
    json: bool,
    /// Validate and report without writing the output PDF
    #[arg(long)]
    dry_run: bool,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct CompressionOfficeOfficeArgs {
    /// Path to the input DOCX/PPTX/XLSX file
    input: PathBuf,
    /// Office format: docx, pptx, or xlsx
    #[arg(long)]
    format: String,
    /// Output JSON report; defaults to stdout
    #[arg(short, long)]
    output: Option<PathBuf>,
}

#[derive(Parser)]
struct CompressionOfficeOfficeToPdfArgs {
    /// Path to the input DOCX/PPTX/XLSX file
    input: PathBuf,
    /// Output PDF
    #[arg(short, long, default_value = "compression_office-office.pdf")]
    output: PathBuf,
    /// Office format: docx, pptx, or xlsx
    #[arg(long)]
    format: String,
    /// Optional JSON report path
    #[arg(long)]
    report: Option<PathBuf>,
    /// Print JSON report to stdout
    #[arg(long)]
    json: bool,
    /// Validate and report without writing the output PDF
    #[arg(long)]
    dry_run: bool,
}

#[derive(Parser)]
struct FormActionPolicySanitizeArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// form action policy policy mode
    #[arg(long, default_value = "remove_javascript_only")]
    policy: String,
    /// Optional complete FormJsSanitizerOptions JSON
    #[arg(long)]
    options: Option<PathBuf>,
    /// Permit a secure mutation closeout signature-policy override when structurally required
    #[arg(long)]
    signature_policy_override: bool,
    /// Output PDF
    #[arg(short, long, default_value = "form-js-sanitized.pdf")]
    output: PathBuf,
    /// Optional JSON report path
    #[arg(long)]
    report: Option<PathBuf>,
    /// Print the report to stdout
    #[arg(long)]
    json: bool,
    /// Inventory/validate without writing output bytes
    #[arg(long)]
    dry_run: bool,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct AdvancedEditingVectorListArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// One-based page number
    #[arg(short, long, default_value_t = 1)]
    page: usize,
    /// Output JSON; defaults to stdout
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct SourceEditingTextSelectionArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// One-based page number
    #[arg(short, long, default_value_t = 1)]
    page: usize,
    /// Source text to resolve to PDF text-showing operators
    #[arg(long = "source-text")]
    source_text: String,
    /// Replacement text used for eligibility and encoding checks
    #[arg(long = "replacement-text")]
    replacement_text: String,
    /// Output JSON; defaults to stdout
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct SourceEditingTextEditArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// One-based page number
    #[arg(short, long, default_value_t = 1)]
    page: usize,
    /// Source text to resolve to PDF text-showing operators
    #[arg(long = "source-text")]
    source_text: String,
    /// Replacement text that must fit the operator-preserving contract
    #[arg(long = "replacement-text")]
    replacement_text: String,
    /// Output PDF
    #[arg(short, long, default_value = "operator-text-edited.pdf")]
    output: PathBuf,
    /// Optional JSON report output
    #[arg(long)]
    report: Option<PathBuf>,
    /// Permit an explicit signature-policy override
    #[arg(long)]
    signature_policy_override: bool,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct SourceEditingPathEditArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// Stable vector/path ID from vector-list
    #[arg(long)]
    id: String,
    /// VectorEditOperation JSON file
    #[arg(long)]
    operation: Option<PathBuf>,
    /// One-based page number
    #[arg(short, long, default_value_t = 1)]
    page: usize,
    /// Output PDF
    #[arg(short, long, default_value = "operator-path-edited.pdf")]
    output: PathBuf,
    /// Optional JSON report
    #[arg(long)]
    report: Option<PathBuf>,
    /// Permit an explicit signature-policy override
    #[arg(long)]
    signature_policy_override: bool,
    /// Shared Form policy: reject, edit-all-uses, or clone-edit-one-instance
    #[arg(long, default_value = "reject")]
    shared_form_policy: String,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct SourceEditingImageArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// One-based page number
    #[arg(short, long, default_value_t = 1)]
    page: usize,
    /// Optional occurrence identifier when the caller already resolved one
    #[arg(long)]
    occurrence: Option<String>,
    /// Output JSON; defaults to stdout
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct EditingTransactionsSceneReportArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// One-based pages to include; repeat --page. Empty means bounded all-pages.
    #[arg(short, long)]
    page: Vec<usize>,
    /// Output JSON; defaults to stdout
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct EditingTransactionsSceneSelectArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// One-based page number
    #[arg(short, long, default_value_t = 1)]
    page: usize,
    /// Stable scene node id
    #[arg(long)]
    id: Option<String>,
    /// Point x coordinate in page user space
    #[arg(long)]
    x: Option<f64>,
    /// Point y coordinate in page user space
    #[arg(long)]
    y: Option<f64>,
    /// Region as x0,y0,x1,y1
    #[arg(long)]
    region: Option<String>,
    /// Cycle through overlapping nodes
    #[arg(long, default_value_t = 0)]
    cycle_index: usize,
    /// Output JSON; defaults to stdout
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct EditingTransactionsTransactionArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// Optional JSON SceneTextEditRequest; otherwise CLI text args are used
    #[arg(long)]
    request: Option<PathBuf>,
    /// One-based page number
    #[arg(short, long, default_value_t = 1)]
    page: usize,
    /// Source text to resolve to source operators
    #[arg(long = "source-text", default_value = "")]
    source_text: String,
    /// Replacement text
    #[arg(long = "replacement-text", default_value = "")]
    replacement_text: String,
    /// Output PDF for apply commands
    #[arg(short, long, default_value = "editing_transactions-scene-edited.pdf")]
    output: PathBuf,
    /// Optional JSON report path
    #[arg(long)]
    report: Option<PathBuf>,
    /// Permit an explicit signature-policy override
    #[arg(long)]
    signature_policy_override: bool,
    /// Font policy
    #[arg(long, default_value = "rebuild_subset_or_generated_type0")]
    font_policy: String,
    /// Direction override: ltr or rtl
    #[arg(long)]
    direction: Option<String>,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct EditingTransactionsTextArgs {
    /// Text to inspect or shape
    text: String,
    /// Direction override: ltr or rtl
    #[arg(long)]
    direction: Option<String>,
    /// Font/subset policy
    #[arg(long)]
    policy: Option<String>,
    /// Output JSON; defaults to stdout
    #[arg(short, long)]
    output: Option<PathBuf>,
}

#[derive(Parser)]
struct EditingTransactionsFontSubstitutionArgs {
    /// Requested font family
    requested_family: String,
    /// Text requiring font coverage
    text: String,
    /// Substitution policy, e.g. allow_substitute
    #[arg(long)]
    policy: Option<String>,
    /// Output JSON; defaults to stdout
    #[arg(short, long)]
    output: Option<PathBuf>,
}

#[derive(Parser)]
struct TextReflowReflowArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// Optional JSON GeometricReflowRequest; otherwise CLI args are used
    #[arg(long)]
    request: Option<PathBuf>,
    /// One-based page number
    #[arg(short, long, default_value_t = 1)]
    page: usize,
    /// Source text to resolve to source operators
    #[arg(long = "source-text", default_value = "")]
    source_text: String,
    /// Replacement text
    #[arg(long = "replacement-text", default_value = "")]
    replacement_text: String,
    /// Requested mode: geometric_block or semantic_document
    #[arg(long, default_value = "geometric_block")]
    mode: String,
    /// Region as x0,y0,x1,y1
    #[arg(long)]
    region: Option<String>,
    /// Language tag for line breaking and hyphenation
    #[arg(long)]
    language: Option<String>,
    /// Direction override: ltr, rtl, or vertical
    #[arg(long)]
    direction: Option<String>,
    /// Font policy: rebuild_subset_or_generated_type0 or preserve_original_per_run
    #[arg(long, default_value = "rebuild_subset_or_generated_type0")]
    font_policy: String,
    /// Enable explicit language hyphenation policy
    #[arg(long)]
    hyphenation: bool,
    /// Allow page creation for SemanticDocument flow
    #[arg(long)]
    allow_page_creation: bool,
    /// Allow review-approved low-confidence semantic structure
    #[arg(long)]
    approve_low_confidence_structure: bool,
    /// Permit an explicit signature-policy override
    #[arg(long)]
    signature_policy_override: bool,
    /// Output PDF for apply commands
    #[arg(short, long, default_value = "text_reflow-reflow-edited.pdf")]
    output: PathBuf,
    /// Optional JSON report path
    #[arg(long)]
    report: Option<PathBuf>,
    /// Output JSON; defaults to stdout for non-apply commands
    #[arg(long = "json-output")]
    json_output: Option<PathBuf>,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct TextReflowStructureCorrectionArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// JSON correction object path
    correction: PathBuf,
    /// Output JSON; defaults to stdout
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct TextReflowValidateArgs {
    /// Original input PDF used for the reflow
    pdf: PathBuf,
    /// Completed reflow output PDF to validate (read-only)
    #[arg(long = "output-pdf")]
    output_pdf: PathBuf,
    /// JSON GeometricReflowRequest used to produce the output
    #[arg(long)]
    request: PathBuf,
    /// Output JSON; defaults to stdout and never overwrites either PDF
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Password for the original encrypted PDF, if required
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct TextReflowUndoArgs {
    /// Original input PDF used for the reflow
    pdf: PathBuf,
    /// Completed reflow output PDF to verify and undo (read-only)
    #[arg(long = "output-pdf")]
    output_pdf: PathBuf,
    /// JSON GeometricReflowRequest used to produce the output
    #[arg(long)]
    request: PathBuf,
    /// Destination for restored PDF bytes; must differ from both inputs
    #[arg(long = "restored-pdf")]
    restored_pdf: PathBuf,
    /// Optional JSON undo report path
    #[arg(long)]
    report: Option<PathBuf>,
    /// Password for the original encrypted PDF, if required
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct DocumentSubsystemsRequestArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// Typed DocumentSubsystemsRequest JSON file
    #[arg(long)]
    request: PathBuf,
    /// Output JSON; defaults to stdout
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct DocumentSubsystemsApplyArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// Typed DocumentSubsystemsRequest JSON file
    #[arg(long)]
    request: PathBuf,
    /// Distinct output PDF; existing files are never overwritten
    #[arg(short, long, default_value = "document_subsystems-edited.pdf")]
    output: PathBuf,
    /// Optional JSON operation report; existing files are never overwritten
    #[arg(long)]
    report: Option<PathBuf>,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct DocumentSubsystemsUndoArgs {
    /// Original input PDF used for document subsystems apply
    pdf: PathBuf,
    /// Edited PDF returned by document_subsystems-apply
    #[arg(long = "output-pdf")]
    output_pdf: PathBuf,
    /// Exact Typed DocumentSubsystemsRequest JSON file used to apply
    #[arg(long)]
    request: PathBuf,
    /// Distinct restored PDF output; existing files are never overwritten
    #[arg(long = "restored-pdf")]
    restored_pdf: PathBuf,
    /// Optional JSON inverse report; existing files are never overwritten
    #[arg(long)]
    report: Option<PathBuf>,
    /// Password for the original encrypted PDF
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct DocumentSecurityRequestArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// Typed DocumentSecurityRequest JSON file
    #[arg(long)]
    request: PathBuf,
    /// Output JSON; defaults to stdout
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct DocumentSecurityApplyArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// Typed DocumentSecurityRequest JSON file
    #[arg(long)]
    request: PathBuf,
    /// Distinct output PDF; existing files are never overwritten
    #[arg(short, long, default_value = "document_security-edited.pdf")]
    output: PathBuf,
    /// Optional JSON operation report; existing files are never overwritten
    #[arg(long)]
    report: Option<PathBuf>,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct DocumentSecurityUndoArgs {
    /// Original input PDF used for document security apply
    pdf: PathBuf,
    /// Edited PDF returned by document_security-apply
    #[arg(long = "output-pdf")]
    output_pdf: PathBuf,
    /// Exact Typed DocumentSecurityRequest JSON file used to apply
    #[arg(long)]
    request: PathBuf,
    /// Distinct restored PDF output; existing files are never overwritten
    #[arg(long = "restored-pdf")]
    restored_pdf: PathBuf,
    /// Optional JSON inverse report; existing files are never overwritten
    #[arg(long)]
    report: Option<PathBuf>,
    /// Password for the original encrypted PDF
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct DocumentSecurityVerifyArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// JSON array of terms to verify as absent
    #[arg(long)]
    terms: PathBuf,
    /// Output JSON; defaults to stdout
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct AdvancedEditingVectorEditArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// Stable vector object ID from vector-list
    #[arg(long)]
    id: String,
    /// VectorEditOperation JSON file
    #[arg(long)]
    operation: PathBuf,
    /// One-based page number
    #[arg(short, long, default_value_t = 1)]
    page: usize,
    /// Output PDF
    #[arg(short, long, default_value = "vector-edited.pdf")]
    output: PathBuf,
    /// Optional JSON report
    #[arg(long)]
    report: Option<PathBuf>,
    /// Permit an explicit signature-policy override
    #[arg(long)]
    signature_policy_override: bool,
    /// Shared Form policy: reject, edit-all-uses, or clone-edit-one-instance
    #[arg(long, default_value = "reject")]
    shared_form_policy: String,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct AdvancedEditingVectorDirectArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// Stable vector object ID from vector-list
    #[arg(long)]
    id: String,
    /// One-based page number
    #[arg(short, long, default_value_t = 1)]
    page: usize,
    /// Duplicate X offset (ignored for delete)
    #[arg(long, default_value_t = 10.0)]
    dx: f64,
    /// Duplicate Y offset (ignored for delete)
    #[arg(long, default_value_t = -10.0)]
    dy: f64,
    /// Output PDF
    #[arg(short, long, default_value = "vector-output.pdf")]
    output: PathBuf,
    /// Optional JSON report
    #[arg(long)]
    report: Option<PathBuf>,
    /// Permit an explicit signature-policy override
    #[arg(long)]
    signature_policy_override: bool,
    /// Shared Form policy: reject, edit-all-uses, or clone-edit-one-instance
    #[arg(long, default_value = "reject")]
    shared_form_policy: String,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct AdvancedEditingCloseoutTextRangeArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// One-based page number used by --analyze
    #[arg(short, long, default_value_t = 1)]
    page: usize,
    /// Analyze the page-local logical range model instead of mutating
    #[arg(long)]
    analyze: bool,
    /// Select by logical offsets supplied in the request JSON
    #[arg(long)]
    logical: bool,
    /// Report that a visual selection has already been resolved to a logical request
    #[arg(long)]
    visual_selection: bool,
    /// Complete MultiRunTextRangeRequest JSON file for a logical edit
    #[arg(long)]
    request: Option<PathBuf>,
    /// Output PDF for an edit
    #[arg(short, long, default_value = "text-range-edited.pdf")]
    output: PathBuf,
    /// Optional JSON report output
    #[arg(long)]
    report: Option<PathBuf>,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct AdvancedEditingInkFitArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// One-based page number
    #[arg(short, long, default_value_t = 1)]
    page: usize,
    /// Zero-based annotation index
    #[arg(long, default_value_t = 0)]
    annotation: usize,
    /// Optional InkFitOptions JSON file
    #[arg(long)]
    options: Option<PathBuf>,
    /// Output PDF
    #[arg(short, long, default_value = "ink-fitted.pdf")]
    output: PathBuf,
    /// Optional JSON report
    #[arg(long)]
    report: Option<PathBuf>,
    /// Permit an explicit signature-policy override
    #[arg(long)]
    signature_policy_override: bool,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct WordPaginationAuditArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// DOCX layout mode: flowing, page-faithful, or hybrid
    #[arg(long, default_value = "page-faithful")]
    layout: String,
    /// Optional JSON report; defaults to stdout
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Optional Microsoft Word executable/automation harness path recorded by the report
    #[arg(long)]
    word: Option<PathBuf>,
    /// Optional LibreOffice executable path recorded by the report
    #[arg(long)]
    libreoffice: Option<PathBuf>,
    /// Return a failure when the structural report contains exact unsupported rows
    #[arg(long)]
    fail_on_unsupported: bool,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct AnnotationXfdfExportArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// Output XFDF
    #[arg(short, long, default_value = "annotations.xfdf")]
    output: PathBuf,
    /// Optional JSON report path
    #[arg(long)]
    report: Option<PathBuf>,
    /// Print JSON report to stdout
    #[arg(long)]
    json: bool,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct AnnotationXfdfImportArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// Annotation XFDF input
    xfdf: PathBuf,
    /// Output PDF
    #[arg(short, long, default_value = "annotations-imported.pdf")]
    output: PathBuf,
    /// Optional JSON import-options file
    #[arg(long)]
    options: Option<PathBuf>,
    /// Optional JSON report path
    #[arg(long)]
    report: Option<PathBuf>,
    /// Print JSON report to stdout
    #[arg(long)]
    json: bool,
    /// Validate/import in memory without writing the PDF
    #[arg(long)]
    dry_run: bool,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct AnnotationAppearanceGenerateArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// Output PDF
    #[arg(short, long, default_value = "annotation-appearances.pdf")]
    output: PathBuf,
    /// Optional JSON options file
    #[arg(long)]
    options: Option<PathBuf>,
    /// Optional JSON report path
    #[arg(long)]
    report: Option<PathBuf>,
    /// Print JSON report to stdout
    #[arg(long)]
    json: bool,
    /// Generate/validate in memory without writing the PDF
    #[arg(long)]
    dry_run: bool,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct AnnotationAppearanceReportArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// Optional JSON options file
    #[arg(long)]
    options: Option<PathBuf>,
    /// Output JSON report; defaults to stdout
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct RichMediaSanitizeArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// Output PDF
    #[arg(short, long, default_value = "rich-media-sanitized.pdf")]
    output: PathBuf,
    /// Policy: inventory_only, preserve_inert, remove_active_content,
    /// remove_all_media, flatten_static_poster, or custom
    #[arg(long, default_value = "remove_active_content")]
    policy: String,
    /// Custom policy JSON (required only for --policy custom)
    #[arg(long)]
    custom: Option<PathBuf>,
    /// Optional JSON report path
    #[arg(long)]
    report: Option<PathBuf>,
    /// Print JSON report to stdout
    #[arg(long)]
    json: bool,
    /// Run and report without writing the output PDF
    #[arg(long)]
    dry_run: bool,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct NonAxisRedactionArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// JSON NonAxisRedactionOptions request file
    plan: PathBuf,
    /// Output PDF
    #[arg(short, long, default_value = "nonaxis-redacted.pdf")]
    output: PathBuf,
    /// Optional JSON report path
    #[arg(long)]
    report: Option<PathBuf>,
    /// Print JSON report to stdout
    #[arg(long)]
    json: bool,
    /// Return only the plan report and do not write a PDF
    #[arg(long)]
    dry_run: bool,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
    /// Promote securely decoded inline images to deterministic Image XObjects
    #[arg(long)]
    promote: bool,
}

#[derive(Parser)]
struct AssociatedFilesExtractArgs {
    pdf: PathBuf,
    /// Stable id from associated-files-report
    id: String,
    #[arg(short, long)]
    output: PathBuf,
    #[arg(long)]
    report: Option<PathBuf>,
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct AssociatedFilesAddArgs {
    pdf: PathBuf,
    /// Payload to embed
    file: PathBuf,
    /// JSON AssociatedFileAddRequest
    options: PathBuf,
    #[arg(short, long, default_value = "associated-files-added.pdf")]
    output: PathBuf,
    #[arg(long)]
    report: Option<PathBuf>,
    #[arg(long)]
    json: bool,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct AssociatedFilesUpdateArgs {
    pdf: PathBuf,
    file: PathBuf,
    /// JSON AssociatedFileOwnerUpdateRequest
    options: PathBuf,
    #[arg(short, long, default_value = "associated-files-updated.pdf")]
    output: PathBuf,
    #[arg(long)]
    report: Option<PathBuf>,
    #[arg(long)]
    json: bool,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct AssociatedFilesRemoveArgs {
    pdf: PathBuf,
    /// Stable ids to remove; repeat --id
    #[arg(long, required = true)]
    id: Vec<String>,
    #[arg(short, long, default_value = "associated-files-removed.pdf")]
    output: PathBuf,
    #[arg(long)]
    report: Option<PathBuf>,
    #[arg(long)]
    json: bool,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    password: Option<String>,
    /// Owner kind for owner-specific unlink (requires exactly one --id)
    #[arg(long)]
    owner: Option<String>,
    /// Exact owner object-generation id
    #[arg(long)]
    owner_ref: Option<String>,
}

#[derive(Parser)]
struct EditFormArgs {
    pdf: PathBuf,
    #[arg(long)]
    field: String,
    #[arg(long)]
    value: String,
    #[arg(long, default_value = "enforce")]
    signature_policy: String,
    #[arg(short, long, default_value = "form-edited-incremental.pdf")]
    output: PathBuf,
    #[arg(long)]
    report: Option<PathBuf>,
    #[arg(long)]
    json: bool,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct SignaturePreservingFormArgs {
    pdf: PathBuf,
    #[arg(long)]
    field: String,
    #[arg(long)]
    value: String,
    /// JSON VerifyOptions object used for pre/post signature validation.
    #[arg(long = "signature-options", value_name = "JSON")]
    signature_options: Option<PathBuf>,
    /// Policy: enforce or override. Override is an explicit invalidation opt-in.
    #[arg(long, default_value = "enforce")]
    signature_policy: String,
    #[arg(short, long, default_value = "signature-preserving-form-edit.pdf")]
    output: PathBuf,
    #[arg(long)]
    report: Option<PathBuf>,
    #[arg(long)]
    json: bool,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct EditMutationArgs {
    pdf: PathBuf,
    /// JSON mutation request
    options: PathBuf,
    #[arg(long, default_value = "enforce")]
    signature_policy: String,
    #[arg(short, long)]
    output: PathBuf,
    #[arg(long)]
    report: Option<PathBuf>,
    #[arg(long)]
    json: bool,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct AssociatedFilesSanitizeArgs {
    pdf: PathBuf,
    /// JSON AssociatedFileSanitizerOptions
    #[arg(long)]
    options: Option<PathBuf>,
    #[arg(short, long, default_value = "associated-files-sanitized.pdf")]
    output: PathBuf,
    #[arg(long)]
    report: Option<PathBuf>,
    #[arg(long)]
    json: bool,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct EditPolicyArgs {
    pdf: PathBuf,
    /// Operation such as form_value_update, annotation_add, redaction, or attachment_remove
    operation: String,
    #[arg(short, long)]
    output: Option<PathBuf>,
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct FormsExportArgs {
    /// Path to the PDF file
    pdf: PathBuf,
    /// Output file. Defaults to stdout.
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Export format: json, fdf, or xfdf
    #[arg(long, default_value = "json")]
    format: String,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct FormsImportArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// Field data file to import
    data: PathBuf,
    /// Output PDF
    #[arg(short, long, alias = "out", default_value = "forms-filled.pdf")]
    output: PathBuf,
    /// Input data format: json, fdf, or xfdf. Defaults to the data extension.
    #[arg(long)]
    format: Option<String>,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
    /// Emit a JSON result summary
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct AnnotationsReportArgs {
    /// Path to the PDF file
    pdf: PathBuf,
    /// Output file, defaults to stdout
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct AnnotationsFlattenArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// Output PDF
    #[arg(
        short,
        long,
        alias = "out",
        default_value = "annotations-flattened.pdf"
    )]
    output: PathBuf,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
    /// Emit a JSON result summary
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct PagesReportArgs {
    /// Path to the PDF file
    pdf: PathBuf,
    /// Output file, defaults to stdout
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct InteractiveReportArgs {
    /// Path to the PDF file
    pdf: PathBuf,
    /// Output file, defaults to stdout
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct RedactArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// Output file
    #[arg(short, long, alias = "out", default_value = "redacted.pdf")]
    output: PathBuf,
    /// Search term to redact. Repeat for multiple terms.
    #[arg(long = "text")]
    text: Vec<String>,
    /// Explicit redaction rectangle as page:x,y,w,h in PDF user-space points.
    /// Repeat for multiple rectangles.
    #[arg(long = "rect")]
    rects: Vec<String>,
    /// Page range used for --text search: all, 1, 2-5, or 1,3,7
    #[arg(short, long, default_value = "all")]
    pages: String,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
    /// Disable metadata/XMP/attachment string scrubbing for removed text.
    #[arg(long)]
    no_metadata_scrub: bool,
    /// Image redaction policy: partial, remove, or fail
    #[arg(long, default_value = "partial")]
    image_policy: String,
    /// Attachment policy: keep, remove-all, or remove-overlapping
    #[arg(long, default_value = "keep")]
    attachments: String,
    /// Emit a JSON result summary.
    #[arg(long)]
    json: bool,
    /// Fail if verification finds any requested term after redaction.
    #[arg(long)]
    strict: bool,
}

#[derive(Parser)]
struct PagesCropArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// Crop rectangle as x,y,w,h in PDF user-space points
    #[arg(long)]
    rect: String,
    /// Page range: all, 1, 2-5, or 1,3,7
    #[arg(short, long, default_value = "all")]
    pages: String,
    /// Output file
    #[arg(short, long, alias = "out", default_value = "cropped.pdf")]
    output: PathBuf,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
    /// Emit a JSON result summary
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct PagesScaleArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// Scale factor. 1.0 preserves visual size.
    #[arg(long)]
    scale: f64,
    /// Page range: all, 1, 2-5, or 1,3,7
    #[arg(short, long, default_value = "all")]
    pages: String,
    /// Rasterization DPI used for the visual page copy
    #[arg(long, default_value = "144")]
    dpi: u32,
    /// Output file
    #[arg(short, long, alias = "out", default_value = "scaled.pdf")]
    output: PathBuf,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
    /// Emit a JSON result summary
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct PagesNupArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// Number of columns per output sheet
    #[arg(long, default_value = "2")]
    columns: usize,
    /// Number of rows per output sheet
    #[arg(long, default_value = "1")]
    rows: usize,
    /// Page range: all, 1, 2-5, or 1,3,7
    #[arg(short, long, default_value = "all")]
    pages: String,
    /// Rasterization DPI used for source pages
    #[arg(long, default_value = "144")]
    dpi: u32,
    /// Output file
    #[arg(short, long, alias = "out", default_value = "nup.pdf")]
    output: PathBuf,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
    /// Emit a JSON result summary
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct ChunkArgs {
    /// Path to the PDF file
    pdf: PathBuf,
    /// Output file, defaults to stdout
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Page range: all, 1, 2-5, or 1,3,7
    #[arg(short, long, default_value = "all")]
    pages: String,
    /// Target chunk size in (estimated) tokens.
    #[arg(long, default_value = "512")]
    target_tokens: usize,
    /// Token overlap carried between consecutive chunks (0 disables).
    #[arg(long, default_value = "64")]
    overlap: usize,
    /// Do NOT prepend the heading hierarchy to each chunk (on by default).
    #[arg(long)]
    no_heading_context: bool,
    /// Do NOT start a new chunk at each heading (on by default).
    #[arg(long)]
    no_split_on_headings: bool,
    /// Keep page furniture (headers/footers/page numbers) in chunk text.
    #[arg(long)]
    keep_furniture: bool,
    /// Emit the Semantic Closeout provenance-aware chunk schema.
    #[arg(long)]
    advanced: bool,
    /// Advanced chunk mode: hybrid, page, section, paragraph, table,
    /// table-row, table-cell, figure-caption, cjk, or search-index.
    #[arg(long, default_value = "hybrid")]
    mode: String,
    /// User-supplied CJK dictionary pack manifest(s). Requires --advanced.
    #[arg(long = "dictionary-pack")]
    dictionary_packs: Vec<PathBuf>,
    /// Output format (currently json only).
    #[arg(short, long, default_value = "json")]
    format: String,
    /// OCR scanned pages first (Tesseract). Requires the `ocr` feature + the
    /// `tesseract` binary; lets chunking work on scanned documents.
    /// Accepts `off`/`auto`/`force`; bare `--ocr` means `auto`.
    #[arg(long, num_args = 0..=1, default_missing_value = "auto")]
    ocr: Option<String>,
    /// OCR languages (Tesseract codes), comma/plus-separated.
    #[arg(long, default_value = "eng")]
    ocr_lang: String,
    /// DPI for OCR rasterization.
    #[arg(long, default_value = "300")]
    ocr_dpi: u32,
    /// Password for encrypted PDFs (the empty user password is tried automatically)
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct SemanticExportArgs {
    /// Path to the PDF file.
    pdf: PathBuf,
    /// Output file, defaults to stdout.
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Page range: all, 1, 2-5, or 1,3,7.
    #[arg(short, long, default_value = "all")]
    pages: String,
    /// View: bundle, summary, semantic, tables, tokens, chunks, search, or status.
    #[arg(long, default_value = "bundle")]
    view: String,
    /// Advanced chunk mode used by bundle/chunks views.
    #[arg(long, default_value = "hybrid")]
    chunk_mode: String,
    /// Target advanced chunk size in estimated tokens.
    #[arg(long, default_value = "512")]
    target_tokens: usize,
    /// Advanced chunk overlap in estimated tokens.
    #[arg(long, default_value = "64")]
    overlap: usize,
    /// User-supplied CJK dictionary pack manifest(s).
    #[arg(long = "dictionary-pack")]
    dictionary_packs: Vec<PathBuf>,
    /// Optional TableFormer/Table Transformer proposal-set JSON to validate and merge.
    #[arg(long)]
    table_proposals: Option<PathBuf>,
    /// Query used by the search view.
    #[arg(long)]
    query: Option<String>,
    /// Password for encrypted PDFs.
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct EvalScoreArgs {
    /// ScoreInput JSON file. If omitted, reads JSON from stdin.
    #[arg(short, long)]
    input: Option<PathBuf>,
    /// Output file for the ScoreOutput JSON; defaults to stdout.
    #[arg(short, long)]
    output: Option<PathBuf>,
}

#[derive(Parser)]
struct ExtractImagesArgs {
    /// Path to the PDF file
    pdf: PathBuf,
    /// Output ZIP file
    #[arg(short, long, default_value = "images.zip")]
    output: PathBuf,
    /// Page range: all, 1, 2-5, or 1,3,7
    #[arg(short, long, default_value = "all")]
    pages: String,
    /// Output format: png, jpg, webp, or original
    #[arg(short, long, default_value = "original")]
    format: String,
    /// JPEG quality 1-100, only for --format jpg
    #[arg(short, long, default_value = "85")]
    quality: u8,
    /// Minimum image width in pixels
    #[arg(long, default_value = "1")]
    min_width: u32,
    /// Minimum image height in pixels
    #[arg(long, default_value = "1")]
    min_height: u32,
    /// Restrict extraction to a page box in PDF user-space points:
    /// x0,y0,x1,y1 with origin bottom-left. Inline images are skipped because
    /// their placement boxes are not yet exposed.
    #[arg(long)]
    region: Option<String>,
    /// Password for an encrypted PDF (the empty user password is tried automatically)
    #[arg(long)]
    password: Option<String>,
    /// Emit a JSON result summary
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct RenderArgs {
    /// Path to the PDF file
    pdf: PathBuf,
    /// Output ZIP file
    #[arg(short, long, default_value = "pages.zip")]
    output: PathBuf,
    /// Page range: all, 1, 2-5, or 1,3,7
    #[arg(short, long, default_value = "all")]
    pages: String,
    /// Resolution in DPI (raster formats; also sets the device scale for svg/ps/eps)
    #[arg(short, long, default_value = "150")]
    dpi: u32,
    /// Output format: png, jpg, webp, svg (vector), ps (PostScript), or eps
    #[arg(short, long, default_value = "png")]
    format: String,
    /// JPEG quality 1-100
    #[arg(short, long, default_value = "85")]
    quality: u8,
    /// Raster compositing mode: compat matches Poppler/Splash; high uses linear-light RGB compositing
    #[arg(long, default_value = "compat", value_parser = ["compat", "high", "high-quality", "hq"])]
    render_quality: String,
    /// Password for an encrypted PDF (the empty user password is tried automatically)
    #[arg(long)]
    password: Option<String>,
    /// Maximum pixels (width*height) per rendered page. A page whose final pixel
    /// count would exceed this is skipped with a clean error instead of attempting
    /// an abusive allocation. Defaults to the engine cap (100 MP); overrides the
    /// WELLFRIENDPDF_MAX_RENDER_PIXELS environment variable when set.
    #[arg(long)]
    max_render_pixels: Option<u64>,
    /// Emit a JSON result summary
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct RenderCompareArgs {
    /// Path to the PDF file
    pdf: PathBuf,
    /// Output report file; defaults to stdout
    #[arg(short, long, alias = "out")]
    output: Option<PathBuf>,
    /// Page range: all, 1, 2-5, or 1,3,7
    #[arg(short, long, default_value = "all")]
    pages: String,
    /// Resolution in DPI used for display-list render verification
    #[arg(short, long, default_value = "72")]
    dpi: u32,
    /// Raster compositing mode: compat matches Poppler/Splash; high uses linear-light RGB compositing
    #[arg(long, default_value = "compat", value_parser = ["compat", "high", "high-quality", "hq"])]
    render_quality: String,
    /// Password for an encrypted PDF (the empty user password is tried automatically)
    #[arg(long)]
    password: Option<String>,
    /// Pretty-print the JSON report
    #[arg(long)]
    pretty: bool,
}

#[derive(Parser)]
struct PdfToJpgArgs {
    /// Path to the PDF file
    pdf: PathBuf,
    /// Directory to write page images into
    #[arg(long, alias = "out", default_value = "pages")]
    out_dir: PathBuf,
    /// Page range: all, 1, 2-5, or 1,3,7
    #[arg(short, long, default_value = "all")]
    pages: String,
    /// Resolution in DPI
    #[arg(short, long, default_value = "150")]
    dpi: u32,
    /// Output format: jpg or png
    #[arg(short, long, default_value = "jpg")]
    format: String,
    /// JPEG quality 1-100
    #[arg(short, long, default_value = "85")]
    quality: u8,
    /// Output filename stem, producing stem-001.jpg / stem-001.png
    #[arg(long, default_value = "page")]
    stem: String,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
    /// Emit a JSON result summary
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct ImageToPdfArgs {
    /// Input image files, one PDF page per image
    #[arg(required = true, num_args = 1..)]
    images: Vec<PathBuf>,
    /// Output PDF file
    #[arg(short, long, alias = "out", default_value = "images.pdf")]
    output: PathBuf,
    /// Page size: a4, letter, or size-to-image
    #[arg(long, default_value = "a4")]
    page_size: String,
    /// Margin in PDF points
    #[arg(long, default_value = "0")]
    margin: f64,
    /// Emit a JSON result summary
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct PdfToXlsxArgs {
    /// Path to the PDF file
    pdf: PathBuf,
    /// Output XLSX file
    #[arg(short, long, alias = "out", default_value = "output.xlsx")]
    output: PathBuf,
    /// Layout policy: pages (one worksheet per PDF page) or tables (one worksheet per table)
    #[arg(long, default_value = "pages", value_parser = ["pages", "tables"])]
    layout: String,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
    /// Emit a JSON result summary
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct PdfToPptxArgs {
    /// Path to the PDF file
    pdf: PathBuf,
    /// Output PPTX file
    #[arg(short, long, alias = "out", default_value = "output.pptx")]
    output: PathBuf,
    /// Do not export decodable image XObjects as picture shapes
    #[arg(long)]
    no_images: bool,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
    /// Emit a JSON result summary
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct PdfToDocxArgs {
    /// Path to the PDF file
    pdf: PathBuf,
    /// Output DOCX file
    #[arg(short, long, alias = "out", default_value = "output.docx")]
    output: PathBuf,
    /// Do not export decodable image XObjects as inline pictures
    #[arg(long)]
    no_images: bool,
    /// DOCX layout mode: flowing, page-faithful, or hybrid
    #[arg(long, default_value = "flowing")]
    layout: String,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
    /// Emit a JSON result summary
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct PdfToHtmlArgs {
    /// Path to the PDF file
    pdf: PathBuf,
    /// Output HTML file
    #[arg(short, long, alias = "out", default_value = "output.html")]
    output: PathBuf,
    /// Page range: all, 1, 2-5, or 1,3,7
    #[arg(short, long, default_value = "all")]
    pages: String,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
    /// Emit a JSON result summary
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct PdfToMarkdownArgs {
    /// Path to the PDF file
    pdf: PathBuf,
    /// Output Markdown file
    #[arg(short, long, alias = "out", default_value = "output.md")]
    output: PathBuf,
    /// Page range: all, 1, 2-5, or 1,3,7
    #[arg(short, long, default_value = "all")]
    pages: String,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
    /// Emit a JSON result summary
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct PdfToJsonArgs {
    /// Path to the PDF file
    pdf: PathBuf,
    /// Output JSON file
    #[arg(short, long, alias = "out", default_value = "output.json")]
    output: PathBuf,
    /// Page range: all, 1, 2-5, or 1,3,7
    #[arg(short, long, default_value = "all")]
    pages: String,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
    /// Emit a JSON result summary
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct ExportEditableModelArgs {
    /// Path to the PDF file
    pdf: PathBuf,
    /// Output editable-model JSON file
    #[arg(short, long, alias = "out", default_value = "editable-model.json")]
    output: PathBuf,
    /// Page range: all, 1, 2-5, or 1,3,7
    #[arg(short, long, default_value = "all")]
    pages: String,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
    /// Emit a JSON result summary
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct EditTextArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// Text to search for and replace
    #[arg(long)]
    query: String,
    /// Replacement text to draw in the matched source region
    #[arg(long)]
    replacement: String,
    /// Edit mode: paragraph-reflow, safe-patch, overlay-fallback, rtl-reflow, vertical-reflow, or same-width-patch
    #[arg(long, default_value = "paragraph-reflow")]
    mode: String,
    /// Insert replacement text at this character offset inside the matched paragraph
    #[arg(long)]
    insert_at: Option<usize>,
    /// Delete a character range inside the matched paragraph, formatted start:end
    #[arg(long)]
    delete_range: Option<String>,
    /// Output PDF file
    #[arg(short, long, alias = "out", default_value = "edited.pdf")]
    output: PathBuf,
    /// Page range used for search: all, 1, 2-5, or 1,3,7
    #[arg(short, long, default_value = "all")]
    pages: String,
    /// Case-insensitive matching
    #[arg(long)]
    ignore_case: bool,
    /// Maximum replacements to apply
    #[arg(long, default_value_t = 1)]
    max_replacements: usize,
    /// Replacement font size in PDF points
    #[arg(long, default_value_t = 12.0)]
    font_size: f64,
    /// Replacement fill color as #RRGGBB
    #[arg(long, default_value = "#000000")]
    color: String,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
    /// Emit a JSON result summary
    #[arg(long)]
    json: bool,
    /// Permit a secure mutation closeout signature-policy override when structurally required
    #[arg(long)]
    signature_policy_override: bool,
}

#[derive(Parser)]
struct SaveIncrementalArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// Output PDF file
    #[arg(short, long, alias = "out", default_value = "incremental.pdf")]
    output: PathBuf,
    /// Page number for the incremental text overlay
    #[arg(short, long, default_value_t = 1)]
    page: usize,
    /// Text to append as an incremental overlay
    #[arg(long)]
    text: String,
    /// X position in PDF user-space points
    #[arg(long, default_value_t = 72.0)]
    x: f64,
    /// Y position in PDF user-space points
    #[arg(long, default_value_t = 72.0)]
    y: f64,
    /// Font size in PDF points
    #[arg(long, default_value_t = 12.0)]
    font_size: f64,
    /// Emit deterministic writer diagnostics in JSON output
    #[arg(long)]
    deterministic: bool,
    /// Fixed PDF date string to report for deterministic save workflows
    #[arg(long)]
    fixed_timestamp: Option<String>,
    /// Password for encrypted PDFs
    #[arg(long)]
    password: Option<String>,
    /// Emit a JSON result summary
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct OfficeToPdfArgs {
    /// Input Office file
    input: PathBuf,
    /// Output PDF file
    #[arg(short, long, alias = "out", default_value = "output.pdf")]
    output: PathBuf,
    /// Emit a JSON result summary
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct AnalyzeArgs {
    /// Path to the PDF file
    pdf: PathBuf,
    /// Output as pretty-printed JSON
    #[arg(long)]
    pretty: bool,
}

#[derive(Parser)]
struct MergeArgs {
    /// Input PDF files, in the order their pages should appear
    #[arg(required = true, num_args = 1..)]
    inputs: Vec<PathBuf>,
    /// Output PDF file
    #[arg(short, long, default_value = "merged.pdf")]
    output: PathBuf,
    /// Passwords for encrypted inputs, comma-separated, positionally matched to
    /// inputs (the empty user password is tried automatically). Fewer passwords
    /// than inputs is fine; missing ones default to empty.
    #[arg(long)]
    passwords: Option<String>,
    /// Emit a JSON result summary
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct SplitArgs {
    /// Path to the PDF file
    pdf: PathBuf,
    /// Output filename pattern; %d is replaced with the page number
    /// (e.g. "page-%d.pdf"). %0Nd zero-pads to width N (e.g. "page-%03d.pdf").
    #[arg(short, long, default_value = "page-%d.pdf")]
    output: String,
    /// First page to emit (1-based). Defaults to the first page.
    #[arg(short = 'f', long)]
    first: Option<usize>,
    /// Last page to emit (1-based). Defaults to the last page.
    #[arg(short = 'l', long)]
    last: Option<usize>,
    /// Password for an encrypted PDF (the empty user password is tried automatically)
    #[arg(long)]
    password: Option<String>,
    /// Emit a JSON result summary
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct ExtractPagesArgs {
    /// Path to the PDF file
    pdf: PathBuf,
    /// Page selection, e.g. "1,3,5-9". Order is preserved and duplicates kept.
    pages: String,
    /// Output PDF file
    #[arg(short, long, default_value = "extracted.pdf")]
    output: PathBuf,
    /// Password for an encrypted PDF (the empty user password is tried automatically)
    #[arg(long)]
    password: Option<String>,
    /// Emit a JSON result summary
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct InfoArgs {
    /// Path to the PDF file
    pdf: PathBuf,
    /// Emit machine-readable JSON instead of the human-readable report
    #[arg(long)]
    json: bool,
    /// Password for an encrypted PDF (the empty user password is tried automatically)
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct FeatureReportArgs {
    /// Pretty-print the shared JSON envelope
    #[arg(long)]
    pretty: bool,
    /// Output report file; defaults to stdout
    #[arg(short, long, alias = "out")]
    output: Option<PathBuf>,
}

#[derive(Parser)]
struct ParserReportArgs {
    /// Path to the PDF file
    pdf: PathBuf,
    /// Parser mode: strict, repair, or audit
    #[arg(long, default_value = "audit")]
    mode: String,
    /// Emit machine-readable JSON (the stable default for parser-report)
    #[arg(long)]
    json: bool,
    /// Emit a concise human-readable report
    #[arg(long)]
    pretty: bool,
    /// Output report file; defaults to stdout
    #[arg(short, long, alias = "out")]
    output: Option<PathBuf>,
    /// Include revision-chain details (accepted for explicit audit selection)
    #[arg(long)]
    include_revisions: bool,
    /// Include Arlington validation details (accepted for explicit audit selection)
    #[arg(long)]
    include_arlington: bool,
    /// Include linearization validation details (accepted for explicit audit selection)
    #[arg(long)]
    include_linearization: bool,
    /// Include repair/carving details (accepted for explicit audit selection)
    #[arg(long)]
    include_repair: bool,
    /// Include source and laziness metrics (accepted for explicit audit selection)
    #[arg(long)]
    include_source_metrics: bool,
    /// Include structured stream decode diagnostics and metrics
    #[arg(long)]
    include_decode: bool,
    /// Include structured color/prepress inventory and diagnostics
    #[arg(long)]
    include_color: bool,
    /// Color validation profile for --include-color: generic, pdfa, or pdfx
    #[arg(long, default_value = "generic")]
    color_profile: String,
    /// Decode limit profile: default, low-memory, or audit
    #[arg(long, default_value = "default")]
    decode_profile: String,
    /// Override per-stream decoded byte cap, in MiB
    #[arg(long)]
    decode_max_stream_mb: Option<u64>,
    /// Override maximum filter-chain depth
    #[arg(long)]
    decode_max_chain_depth: Option<usize>,
    /// Override maximum image pixels, in megapixels
    #[arg(long)]
    decode_max_image_mpixels: Option<u64>,
    /// Override decoded-stream cache budget, in MiB
    #[arg(long)]
    decode_cache_mb: Option<u64>,
    /// Override scheduler memory-token budget, in MiB
    #[arg(long)]
    decode_scheduler_mb: Option<u64>,
    /// Maximum diagnostics to emit in the top-level diagnostics array
    #[arg(long)]
    max_diagnostics: Option<usize>,
    /// Exit non-zero if diagnostics reach this severity: error, fatal, or never
    #[arg(long, default_value = "never")]
    fail_on: String,
    /// Password for an encrypted PDF (the empty user password is tried automatically)
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct FontsArgs {
    /// Path to the PDF file
    pdf: PathBuf,
    /// Emit machine-readable JSON instead of the human-readable table
    #[arg(long)]
    json: bool,
    /// Password for an encrypted PDF (the empty user password is tried automatically)
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct DetachArgs {
    /// Path to the PDF file
    pdf: PathBuf,
    /// List embedded files (the default action when no save flag is given)
    #[arg(long)]
    list: bool,
    /// Save the attachment with this 1-based index (from --list)
    #[arg(long, value_name = "N")]
    save: Option<usize>,
    /// Save the attachment with this (original) file name
    #[arg(long, value_name = "NAME")]
    name: Option<String>,
    /// Save every attachment
    #[arg(long)]
    save_all: bool,
    /// Directory to write extracted files into (filenames are sanitized)
    #[arg(long, default_value = ".")]
    output_dir: PathBuf,
    /// Emit machine-readable JSON (for --list)
    #[arg(long)]
    json: bool,
    /// Password for an encrypted PDF (the empty user password is tried automatically)
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct ToHtmlArgs {
    /// Path to the PDF file
    pdf: PathBuf,
    /// Output HTML/XML file (defaults to stdout)
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Page range: all, 1, 2-5, or 1,3,7
    #[arg(short, long, default_value = "all")]
    pages: String,
    /// Complex mode: absolutely-positioned text (default)
    #[arg(long)]
    complex: bool,
    /// Simple mode: flowing paragraphs (lower fidelity, readable)
    #[arg(long)]
    simple: bool,
    /// XML mode: positioned text fragments (pdftohtml -xml)
    #[arg(long)]
    xml: bool,
    /// Complex mode: render the page to a PNG behind the text for full fidelity
    #[arg(long)]
    background: bool,
    /// With --background, make the overlaid text invisible (selectable only)
    #[arg(long)]
    invisible_text: bool,
    /// DPI for the raster background (with --background)
    #[arg(long, default_value = "150")]
    background_dpi: u32,
    /// Password for an encrypted PDF (the empty user password is tried automatically)
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct VerifySigArgs {
    /// Path to the PDF file
    pdf: PathBuf,
    /// Emit machine-readable JSON
    #[arg(long)]
    json: bool,
    /// DER or PEM certificate to trust as a root or pinned signer. Repeatable.
    #[arg(long = "trust-anchor")]
    trust_anchors: Vec<PathBuf>,
    /// DER or PEM untrusted intermediate certificate. Repeatable.
    #[arg(long = "intermediate")]
    intermediates: Vec<PathBuf>,
    /// SHA-256 fingerprint of a certificate that must not appear in a selected
    /// signer, intermediate, or trust-anchor path. Repeatable.
    #[arg(long = "distrust-certificate-sha256", value_name = "SHA256")]
    distrust_certificate_sha256: Vec<String>,
    /// DER OCSP response supplied for offline revocation evaluation. Repeatable.
    #[arg(long = "ocsp")]
    ocsp_responses: Vec<PathBuf>,
    /// DER CRL supplied for offline revocation evaluation. Repeatable.
    #[arg(long = "crl")]
    crls: Vec<PathBuf>,
    /// Validation time as Unix seconds. Defaults to the system clock.
    #[arg(long)]
    validation_time_unix: Option<u64>,
    /// Revocation mode: not-checked, offline-strict, offline-best-effort,
    /// online-strict, or online-best-effort. Online modes require --online
    /// unless supplied/replayed evidence already establishes the decision.
    #[arg(long, default_value = "not-checked")]
    revocation: String,
    /// JSON file containing the shared SignatureAlgorithmPolicy object.
    #[arg(long = "algorithm-policy", value_name = "JSON")]
    algorithm_policy: Option<PathBuf>,
    /// Import a content-addressed Signature Validation evidence bundle for offline replay.
    #[arg(long = "evidence-in", value_name = "JSON")]
    evidence_in: Option<PathBuf>,
    /// Export cryptographically accepted path/revocation evidence to a new
    /// content-addressed bundle. This never enables network retrieval by itself.
    #[arg(long = "evidence-out", value_name = "JSON")]
    evidence_out: Option<PathBuf>,
    /// Opt in to bounded HTTP/HTTPS AIA, OCSP, and CRL retrieval.
    #[arg(long)]
    online: bool,
    /// Allow only these evidence hosts when --online is set. Repeatable.
    #[arg(long = "network-allow-host", value_name = "HOST")]
    network_allow_hosts: Vec<String>,
    /// Per-request total network deadline in milliseconds when --online is set.
    #[arg(long = "network-timeout-ms", value_name = "MS")]
    network_timeout_ms: Option<u64>,
    /// Maximum bytes accepted from one network evidence response when --online is set.
    #[arg(long = "network-max-response-bytes", value_name = "BYTES")]
    network_max_response_bytes: Option<usize>,
    /// Directory for an atomic cache of cryptographically accepted AIA/OCSP/CRL
    /// evidence. Requires --online and is never enabled implicitly.
    #[arg(long = "cache-dir", value_name = "DIR")]
    cache_dir: Option<PathBuf>,
    /// Require a fresh OCSP nonce and reject responses that do not echo it.
    /// This is an explicit online-only policy because many responders omit nonces.
    #[arg(long = "ocsp-require-nonce")]
    ocsp_require_nonce: bool,
    /// Password for an encrypted PDF (the empty user password is tried automatically)
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct TimestampVerifyArgs {
    /// DER-encoded RFC 3161 TimeStampToken
    token: PathBuf,
    /// File containing the exact CMS SignerInfo.signature octets being timestamped
    #[arg(long = "signature-value", value_name = "BYTES")]
    signature_value: PathBuf,
    /// Emit machine-readable JSON
    #[arg(long)]
    json: bool,
    /// DER or PEM certificate to trust as a TSA root. Repeatable.
    #[arg(long = "trust-anchor")]
    trust_anchors: Vec<PathBuf>,
    /// DER or PEM untrusted intermediate certificate. Repeatable.
    #[arg(long = "intermediate")]
    intermediates: Vec<PathBuf>,
    /// SHA-256 fingerprint of a certificate that must not appear in a selected TSA path.
    #[arg(long = "distrust-certificate-sha256", value_name = "SHA256")]
    distrust_certificate_sha256: Vec<String>,
    /// DER OCSP response supplied for offline TSA revocation evaluation. Repeatable.
    #[arg(long = "ocsp")]
    ocsp_responses: Vec<PathBuf>,
    /// DER CRL supplied for offline TSA revocation evaluation. Repeatable.
    #[arg(long = "crl")]
    crls: Vec<PathBuf>,
    /// Validation time as Unix seconds for non-timestamp policy metadata.
    /// The TSA certificate path is evaluated at TSTInfo.genTime.
    #[arg(long)]
    validation_time_unix: Option<u64>,
    /// Revocation mode: not-checked, offline-strict, offline-best-effort,
    /// online-strict, or online-best-effort.
    #[arg(long, default_value = "not-checked")]
    revocation: String,
    /// JSON file containing the shared SignatureAlgorithmPolicy object.
    #[arg(long = "algorithm-policy", value_name = "JSON")]
    algorithm_policy: Option<PathBuf>,
    /// Import a Signature Validation/25 evidence bundle for offline replay.
    #[arg(long = "evidence-in", value_name = "JSON")]
    evidence_in: Option<PathBuf>,
    /// Opt in to bounded HTTP/HTTPS AIA, OCSP, and CRL retrieval.
    #[arg(long)]
    online: bool,
    /// Allow only these evidence hosts when --online is set. Repeatable.
    #[arg(long = "network-allow-host", value_name = "HOST")]
    network_allow_hosts: Vec<String>,
    /// Per-request total network deadline in milliseconds when --online is set.
    #[arg(long = "network-timeout-ms", value_name = "MS")]
    network_timeout_ms: Option<u64>,
    /// Maximum bytes accepted from one network evidence response when --online is set.
    #[arg(long = "network-max-response-bytes", value_name = "BYTES")]
    network_max_response_bytes: Option<usize>,
    /// Directory for an atomic cache of cryptographically accepted AIA/OCSP/CRL evidence.
    #[arg(long = "cache-dir", value_name = "DIR")]
    cache_dir: Option<PathBuf>,
    /// Require a fresh OCSP nonce and reject responses that do not echo it.
    #[arg(long = "ocsp-require-nonce")]
    ocsp_require_nonce: bool,
}

#[derive(Parser)]
struct SecurityReportArgs {
    /// Path to the PDF file
    pdf: PathBuf,
    /// Emit machine-readable JSON
    #[arg(long)]
    json: bool,
    /// Password for an encrypted PDF (the empty user password is tried automatically)
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct SanitizeArgs {
    /// Path to the PDF file
    pdf: PathBuf,
    /// Output sanitized PDF
    #[arg(short, long, default_value = "sanitized.pdf")]
    output: PathBuf,
    /// Policy: strict, balanced, or preserve-visual
    #[arg(long, default_value = "balanced")]
    policy: String,
    /// Emit machine-readable JSON report
    #[arg(long)]
    json: bool,
    /// Fail if risky content remains after sanitization
    #[arg(long)]
    strict: bool,
    /// Password for an encrypted PDF (the empty user password is tried automatically)
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct ValidateArgs {
    /// Path to the PDF file
    pdf: PathBuf,
    /// Profile: pdfa, pdfua, pdfx, security, or all
    #[arg(long, default_value = "all")]
    profile: String,
    /// Emit machine-readable JSON
    #[arg(long)]
    json: bool,
    /// Exit non-zero at this severity: never, error, or warning
    #[arg(long, default_value = "never")]
    fail_on: String,
    /// Exit non-zero on warning as well as error
    #[arg(long)]
    fail_on_warning: bool,
    /// Password for an encrypted PDF (the empty user password is tried automatically)
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct StandardsValidateArgs {
    /// Path to the PDF file
    pdf: PathBuf,
    /// Target profile label, e.g. PDF/A-2B, PDF/UA-1, PDF/X-4 (default: detected/claimed)
    #[arg(long)]
    target: Option<String>,
    /// Emit machine-readable JSON
    #[arg(long)]
    json: bool,
    /// Also write the JSON report to this file
    #[arg(long)]
    output_json: Option<PathBuf>,
    /// Exit non-zero at this severity: never, error, or warning
    #[arg(long, default_value = "never")]
    fail_on: String,
    /// Exit non-zero on warning as well as error
    #[arg(long)]
    fail_on_warning: bool,
    /// Password for an encrypted PDF (the empty user password is tried automatically)
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct SignatureSignArgs {
    /// Path to the input PDF
    pdf: PathBuf,
    /// Output signed PDF (ignored by signature-plan-placeholder)
    #[arg(short, long, default_value = "signed.pdf")]
    output: PathBuf,
    /// Signer private key (PEM: PKCS#8 or PKCS#1)
    #[arg(long)]
    key: PathBuf,
    /// Signer certificate (PEM)
    #[arg(long)]
    cert: PathBuf,
    /// Additional issuer chain certificate(s) (PEM); repeatable
    #[arg(long)]
    chain: Vec<PathBuf>,
    /// Reserve N bytes for the CMS /Contents placeholder
    #[arg(long, default_value_t = 16384)]
    placeholder_size: usize,
    /// Create a certification (DocMDP) signature with permission level 1, 2, or 3
    #[arg(long)]
    certify: Option<u8>,
    /// Signature field name (/T)
    #[arg(long)]
    field_name: Option<String>,
    /// Signature reason (/Reason)
    #[arg(long)]
    reason: Option<String>,
    /// Emit machine-readable JSON
    #[arg(long)]
    json: bool,
    /// Overwrite the output file if it exists
    #[arg(long)]
    force: bool,
    /// Password for an encrypted PDF (signing encrypted inputs is not supported)
    #[arg(long)]
    password: Option<String>,
}

#[derive(Parser)]
struct CanonicalizeArgs {
    /// Path to the PDF file
    pdf: PathBuf,
    /// Output deterministic full-rewrite PDF
    #[arg(short, long, default_value = "canonical.pdf")]
    output: PathBuf,
    /// Emit machine-readable JSON report
    #[arg(long)]
    json: bool,
    /// Fixed SOURCE_DATE_EPOCH-like value for audit reports
    #[arg(long)]
    source_date_epoch: Option<i64>,
    /// Password for an encrypted PDF (the empty user password is tried automatically)
    #[arg(long)]
    password: Option<String>,
}

fn main() -> ExitCode {
    match std::thread::Builder::new()
        .name("wellfriendpdf-cli".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(run_cli)
    {
        Ok(handle) => handle.join().unwrap_or_else(|_| {
            eprintln!(
                "wellfriendpdf: internal error: command panicked; this is a bug, not a PDF-level error"
            );
            ExitCode::from(CliExitCode::Internal.code())
        }),
        Err(err) => {
            eprintln!("wellfriendpdf: internal error: could not start CLI worker thread: {err}");
            ExitCode::from(CliExitCode::Internal.code())
        }
    }
}

fn run_cli() -> ExitCode {
    let cli = Cli::parse();

    let default_panic_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| dispatch(cli)));
    std::panic::set_hook(default_panic_hook);

    match result {
        Ok(Ok(())) => ExitCode::from(CliExitCode::Success.code()),
        Ok(Err(err)) => {
            let code = classify_error(err.as_ref());
            eprintln!("wellfriendpdf: {}: {}", code.label(), err);
            ExitCode::from(code.code())
        }
        Err(_) => {
            eprintln!(
                "wellfriendpdf: internal error: command panicked; this is a bug, not a PDF-level error"
            );
            ExitCode::from(CliExitCode::Internal.code())
        }
    }
}

fn dispatch(cli: Cli) -> Result<(), Box<dyn Error>> {
    let runtime_config_json = runtime_config_json_from_cli(&cli)?;
    match cli.command {
        Commands::Capabilities(args) => {
            run_runtime_capabilities(args, runtime_config_json.as_deref())
        }
        Commands::RuntimeConfig(args) => run_runtime_config(args, runtime_config_json.as_deref()),
        Commands::Providers(args) => run_runtime_providers(args, runtime_config_json.as_deref()),
        Commands::ExtractText(args) => run_extract_text(args),
        Commands::ExtractTables(args) => run_extract_tables(args),
        Commands::Parse(args) => run_parse(args),
        Commands::DocumentModel(args) => run_document_model(args),
        Commands::ExtractFields(args) => run_extract_fields(args),
        Commands::FormsReport(args) => run_forms_report(args),
        Commands::XfaReport(args) => run_xfa_report(args),
        Commands::XfaExtract(args) => run_xfa_extract(args),
        Commands::XfaRender(args) => run_xfa_render(args),
        Commands::XfaFlatten(args) => run_xfa_flatten(args),
        Commands::XfaSanitize(args) => run_xfa_sanitize(args),
        Commands::XfaScriptReport(args) => run_xfa_script_report(args),
        Commands::XfaSecurityReport(args) => run_xfa_security_report(args),
        Commands::XfaRuntimeReport(args) => run_xfa_runtime_report(args),
        Commands::FormsExport(args) => run_forms_export(args),
        Commands::FormsImport(args) => run_forms_import(args),
        Commands::AnnotationsReport(args) => run_annotations_report(args),
        Commands::AnnotationXfdfExport(args) => run_annotation_xfdf_export(args),
        Commands::AnnotationXfdfImport(args) => run_annotation_xfdf_import(args),
        Commands::AnnotationAppearanceGenerate(args) => run_annotation_appearance_generate(args),
        Commands::AnnotationAppearanceReport(args) => run_annotation_appearance_report(args),
        Commands::RichMediaReport(args) => run_rich_media_report(args),
        Commands::RichMediaSanitize(args) => run_rich_media_sanitize(args),
        Commands::RichMediaFlattenPoster(args) => run_rich_media_flatten_poster(args),
        Commands::RedactImageNonaxis(args) => run_nonaxis_redaction(args),
        Commands::AnnotationMediaRedactionReport(args) => {
            run_annotation_media_redaction_report(args)
        }
        Commands::RedactImageMask(args) => run_secure_mutation_redaction(args, true),
        Commands::RedactInlineImage(args) => run_secure_mutation_redaction(args, false),
        Commands::AssociatedFilesReport(args) => run_associated_files_report(args),
        Commands::AssociatedFilesExtract(args) => run_associated_files_extract(args),
        Commands::AssociatedFilesAdd(args) => run_associated_files_add(args),
        Commands::AssociatedFilesUpdate(args) => run_associated_files_update(args),
        Commands::AssociatedFilesRemove(args) => run_associated_files_remove(args),
        Commands::AssociatedFilesSanitize(args) => run_associated_files_sanitize(args),
        Commands::EditSignatureImpact(args) => run_edit_policy_report(args, true),
        Commands::EditPolicyReport(args) => run_edit_policy_report(args, false),
        Commands::SecureMutationReport(args) => run_secure_mutation_report(args),
        Commands::SecureMutationCloseoutReport(args) => run_secure_mutation_closeout_report(args),
        Commands::FormJsReport(args) => run_form_js_report(args),
        Commands::FormJsSanitize(args) => run_form_js_sanitize(args, false),
        Commands::FormJsFlattenValues(args) => run_form_js_sanitize(args, true),
        Commands::InteractiveDataReport(args) => run_interactive_data_closeout_report(args),
        Commands::WordPaginationAudit(args) => run_word_pagination_audit(args),
        Commands::FormActionPolicyReport(args) => run_form_action_policy_report(args),
        Commands::AdvancedEditingReport(args) => run_advanced_editing_report(args),
        Commands::AdvancedEditingCloseoutReport(args) => run_advanced_editing_closeout_report(args),
        Commands::WriterHistoryReport(args) => run_writer_history_report(args),
        Commands::CompressionOfficeReport(args) => run_compression_office_report(args),
        Commands::CryptoWriterReport(args) => run_crypto_writer_report(args),
        Commands::WriterDeterminismAudit(args) => run_writer_determinism_audit(args),
        Commands::WriterExternalDiff(args) => run_writer_external_diff(args),
        Commands::WriterCloseoutReport(args) => run_writer_closeout_report(args),
        Commands::PubsecReport(args) => run_pubsec_report(args),
        Commands::PubsecDecrypt(args) => run_pubsec_decrypt(args),
        Commands::PubsecEncrypt(args) => run_pubsec_encrypt(args),
        Commands::PubsecAddRecipient(args) => {
            run_pubsec_reencrypt_with_operation(args, "pubsec_add_recipient")
        }
        Commands::PubsecRemoveRecipient(args) => {
            run_pubsec_reencrypt_with_operation(args, "pubsec_remove_recipient")
        }
        Commands::PubsecReplaceRecipient(args) => {
            run_pubsec_reencrypt_with_operation(args, "pubsec_replace_recipient")
        }
        Commands::PubsecReencrypt(args) => run_pubsec_reencrypt(args),
        Commands::PubsecDecryptEditReencrypt(args) => {
            run_pubsec_reencrypt_with_operation(args, "pubsec_decrypt_edit_reencrypt")
        }
        Commands::PdfMacReport(args) => run_pdf_mac_report(args),
        Commands::PdfMacVerify(args) => run_pdf_mac_verify(args),
        Commands::PdfMacCreate(args) => run_pdf_mac_create(args),
        Commands::AesGcmReport(args) => run_aes_gcm_report(args),
        Commands::AesGcmDecrypt(args) => run_aes_gcm_decrypt(args),
        Commands::AesGcmEncrypt(args) => run_aes_gcm_encrypt(args),
        Commands::CryptoTamperTest(args) => run_crypto_tamper_test(args),
        Commands::RasterVectorReport(args) => run_writer_history_raster_vector_report(args),
        Commands::RasterVectorize(args) => run_writer_history_raster_vector_report(args),
        Commands::FontReconstruct(args) => run_writer_history_font_reconstruction_report(args),
        Commands::FontReconstructionReport(args) => {
            run_writer_history_font_reconstruction_report(args)
        }
        Commands::HistoryReport(args) => run_writer_history_history_report(args),
        Commands::HistorySnapshot(args) => run_writer_history_history_report(args),
        Commands::HistoryRestore(args) => run_writer_history_history_report(args),
        Commands::HistoryDiff(args) => run_writer_history_history_report(args),
        Commands::ObjectStreamReport(args) => run_writer_history_object_stream_report(args),
        Commands::SaveObjectStreams(args) => run_writer_history_save_object_streams(args),
        Commands::CompressionOfficeOptimize(args) => run_compression_office_optimize(args),
        Commands::CompressionOfficeOfficeInspect(args) => {
            run_compression_office_office_inspect(args)
        }
        Commands::CompressionOfficeOfficeToPdf(args) => run_compression_office_office_to_pdf(args),
        Commands::EditTextRange(args) => run_advanced_editing_closeout_text_range(args),
        Commands::ProvenanceReport(args) => run_source_editing_provenance(args),
        Commands::EditEligibility(args) => run_source_editing_eligibility(args),
        Commands::EditTextOperator(args) => run_source_editing_text_edit(args),
        Commands::EditPathOperator(args) => run_source_editing_path_edit(args),
        Commands::EditImageOccurrence(args) => run_source_editing_image_eligibility(args),
        Commands::EditFormOccurrence(mut args) => {
            args.shared_form_policy = "clone-edit-one-instance".to_string();
            run_source_editing_path_edit(args)
        }
        Commands::EditOperationReport(args) => run_source_editing_report(args),
        Commands::EditingTransactionsReport(args) => run_editing_transactions_report(args),
        Commands::SceneReport(args) => run_editing_transactions_scene_report(args),
        Commands::SceneSelect(args) => run_editing_transactions_scene_select(args),
        Commands::TransactionPlan(args) => run_editing_transactions_transaction_plan(args),
        Commands::TransactionApply(args) => run_editing_transactions_transaction_apply(args),
        Commands::TransactionUndo(args) => run_editing_transactions_transaction_undo(args),
        Commands::TextMap(args) => run_editing_transactions_text_map(args),
        Commands::ShapeText(args) => run_editing_transactions_shape_text(args),
        Commands::FontSubsetPlan(args) => run_editing_transactions_font_subset_plan(args),
        Commands::FontSubsetBuild(args) => run_editing_transactions_font_subset_plan(args),
        Commands::FontSubstitutionReport(args) => {
            run_editing_transactions_font_substitution_report(args)
        }
        Commands::SceneEditText(args) => run_editing_transactions_transaction_apply(args),
        Commands::TextReflowReport(args) => run_text_reflow_report(args),
        Commands::LayoutAnalyze(args) => run_text_reflow_layout_analyze(args),
        Commands::ReadingOrderReport(args) => run_text_reflow_reading_order_report(args),
        Commands::FlowGraphReport(args) => run_text_reflow_flow_graph_report(args),
        Commands::ReflowPreview(args) => run_text_reflow_reflow_preview(args),
        Commands::OverflowReport(args) => run_text_reflow_overflow_report(args),
        Commands::ReflowConstraints(args) => run_text_reflow_constraints_report(args),
        Commands::ReflowConfidence(args) => run_text_reflow_confidence_report(args),
        Commands::ReflowValidate(args) => run_text_reflow_reflow_validate(args),
        Commands::ReflowRegion(args) => run_text_reflow_reflow_region(args),
        Commands::ReflowDocument(args) => run_text_reflow_reflow_document(args),
        Commands::ReflowUndo(args) => run_text_reflow_reflow_undo(args),
        Commands::ReflowApproveStructure(args) => run_text_reflow_reflow_approve_structure(args),
        Commands::ReflowOperationReport(args) => run_text_reflow_reflow_operation_report(args),
        Commands::DocumentSubsystemsReport(args) => run_document_subsystems_report(args),
        Commands::DocumentSubsystemsAnalyze(args) => run_document_subsystems_analyze(args),
        Commands::DocumentSubsystemsPlan(args) => run_document_subsystems_plan(args),
        Commands::DocumentSubsystemsApply(args) => run_document_subsystems_apply(args),
        Commands::DocumentSubsystemsUndo(args) => run_document_subsystems_undo(args),
        Commands::DocumentSecurityReport(args) => run_document_security_report(args),
        Commands::DocumentSecurityAnalyze(args) => run_document_security_analyze(args),
        Commands::DocumentSecurityPlan(args) => run_document_security_plan(args),
        Commands::DocumentSecurityApply(args) => run_document_security_apply(args),
        Commands::DocumentSecurityUndo(args) => run_document_security_undo(args),
        Commands::DocumentSecurityVerifyResidual(args) => {
            run_document_security_verify_residual(args)
        }
        Commands::FormInstanceReport(args) => run_advanced_editing_vector_list(args),
        Commands::FormCloneOne(mut args) => {
            args.shared_form_policy = "clone-edit-one-instance".to_string();
            run_advanced_editing_vector_edit(args)
        }
        Commands::AnnotationAppearanceSharedReport(args) => run_advanced_editing_vector_list(args),
        Commands::AnnotationAppearanceCloneOne(mut args) => {
            args.shared_form_policy = "clone-edit-one-instance".to_string();
            run_advanced_editing_vector_edit(args)
        }
        Commands::VectorList(args) => run_advanced_editing_vector_list(args),
        Commands::VectorEdit(args) => run_advanced_editing_vector_edit(args),
        Commands::VectorDelete(args) => run_advanced_editing_vector_direct(args, false),
        Commands::VectorDuplicate(args) => run_advanced_editing_vector_direct(args, true),
        Commands::InkFit(args) => run_advanced_editing_ink_fit(args),
        Commands::EditForm(args) => run_edit_form(args),
        Commands::EditAnnotation(args) => run_edit_mutation(args, true),
        Commands::EditPageProperty(args) => run_edit_mutation(args, false),
        Commands::AnnotationsFlatten(args) => run_annotations_flatten(args),
        Commands::PagesReport(args) => run_pages_report(args),
        Commands::InteractiveReport(args) => run_interactive_report(args),
        Commands::Redact(args) => run_redact(args),
        Commands::Chunk(args) => run_chunk(args),
        Commands::SemanticExport(args) => run_semantic_export(args),
        Commands::EvalScore(args) => run_eval_score(args),
        Commands::ExtractImages(args) => run_extract_images(args),
        Commands::Render(args) => run_render(args),
        Commands::RenderCompare(args) => run_render_compare(args),
        Commands::PdfToJpg(args) => run_pdf_to_jpg(args),
        Commands::ImageToPdf(args) => run_image_to_pdf(args),
        Commands::PdfToXlsx(args) => run_pdf_to_xlsx(args),
        Commands::PdfToPptx(args) => run_pdf_to_pptx(args),
        Commands::PdfToDocx(args) => run_pdf_to_docx(args),
        Commands::PdfToHtml(args) => run_pdf_to_html(args),
        Commands::PdfToMarkdown(args) => run_pdf_to_markdown(args),
        Commands::PdfToJson(args) => run_pdf_to_json(args),
        Commands::ExportEditableModel(args) => run_export_editable_model(args),
        Commands::EditText(args) => run_edit_text(args),
        Commands::SaveIncremental(args) => run_save_incremental(args),
        Commands::DocxToPdf(args) => run_docx_to_pdf(args),
        Commands::XlsxToPdf(args) => run_xlsx_to_pdf(args),
        Commands::PptxToPdf(args) => run_pptx_to_pdf(args),
        Commands::Analyze(args) => run_analyze(args),
        Commands::Merge(args) => run_merge(args),
        Commands::Split(args) => run_split(args),
        Commands::ExtractPages(args) => run_extract_pages(args),
        Commands::Info(args) => run_info(args),
        Commands::FeatureReport(args) => run_feature_report(args),
        Commands::ParserReport(args) => run_parser_report(args),
        Commands::CodecIsolationReport(args) => run_codec_isolation_report(args),
        Commands::Fonts(args) => run_fonts(args),
        Commands::Detach(args) => run_detach(args),
        Commands::ToHtml(args) => run_to_html(args),
        Commands::VerifySig(args) => run_verify_sig(args, SignatureCliMode::LegacyInspect),
        Commands::SignatureList(args) => run_verify_sig(args, SignatureCliMode::List),
        Commands::SignatureVerify(args) => run_verify_sig(args, SignatureCliMode::SignatureVerify),
        Commands::PadesVerify(args) => run_verify_sig(args, SignatureCliMode::PadesVerify),
        Commands::SignaturePreservingPlan(args) => run_signature_preserving_form(args, true),
        Commands::SignaturePreservingEdit(args) => run_signature_preserving_form(args, false),
        Commands::TimestampVerify(args) => run_timestamp_verify(args),
        Commands::SignatureTimestamps(args) => {
            run_verify_sig(args, SignatureCliMode::SignatureTimestamps)
        }
        Commands::DssInspect(args) => run_verify_sig(args, SignatureCliMode::DssInspect),
        Commands::LtvVerify(args) => run_verify_sig(args, SignatureCliMode::LtvVerify),
        Commands::PadesLevelReport(args) => {
            run_verify_sig(args, SignatureCliMode::PadesLevelReport)
        }
        Commands::CertificatePathBuild(args) => {
            run_verify_sig(args, SignatureCliMode::CertificatePathBuild)
        }
        Commands::CertificatePathVerify(args) => {
            run_verify_sig(args, SignatureCliMode::CertificatePathVerify)
        }
        Commands::OcspCheck(args) => run_verify_sig(args, SignatureCliMode::OcspCheck),
        Commands::CrlCheck(args) => run_verify_sig(args, SignatureCliMode::CrlCheck),
        Commands::RevocationCheck(args) => run_verify_sig(args, SignatureCliMode::RevocationCheck),
        Commands::EvidenceFetch(args) => run_evidence_fetch(args),
        Commands::EvidenceExport(args) => run_evidence_export(args),
        Commands::EvidenceVerify(args) => run_evidence_replay(args),
        Commands::EvidenceReplay(args) => run_evidence_replay(args),
        Commands::SecurityReport(args) => run_security_report(args),
        Commands::SignatureReport(args) => run_verify_sig(args, SignatureCliMode::LegacyInspect),
        Commands::Sanitize(args) => run_sanitize(args),
        Commands::Validate(args) => run_validate(args),
        Commands::PdfaValidate(args) => run_standards_validate(args, StandardsKind::PdfA),
        Commands::PdfuaValidate(args) => run_standards_validate(args, StandardsKind::PdfUa),
        Commands::PdfxValidate(args) => run_standards_validate(args, StandardsKind::PdfX),
        Commands::StandardsValidate(args) => run_standards_validate(args, StandardsKind::All),
        Commands::SignatureSign(args) => run_signature_sign(args, false),
        Commands::SignaturePlanPlaceholder(args) => run_signature_sign(args, true),
        Commands::Canonicalize(args) => run_canonicalize(args),
        Commands::Encrypt(args) => run_encrypt(args),
        Commands::Decrypt(args) => run_decrypt(args),
        Commands::Watermark(args) => run_watermark(args),
        Commands::AddPageNumbers(args) => run_page_numbers(args),
        Commands::Organize(args) => run_organize(args),
        Commands::Rotate(args) => run_rotate(args),
        Commands::PagesCrop(args) => run_pages_crop(args),
        Commands::PagesScale(args) => run_pages_scale(args),
        Commands::PagesNup(args) => run_pages_nup(args),
        Commands::Optimize(args) => run_optimize(args),
        Commands::Repair(args) => run_repair(args),
        Commands::Linearize(args) => run_linearize(args),
    }
}

fn runtime_config_json_from_cli(cli: &Cli) -> Result<Option<String>, Box<dyn Error>> {
    let mut cfg = match &cli.runtime_config_file {
        Some(path) => wellfriendpdf_engine::RuntimeConfig::from_path(path)?,
        None => wellfriendpdf_engine::RuntimeConfig::standard(),
    };
    if let Some(env_cfg) = wellfriendpdf_engine::RuntimeConfig::from_env()? {
        cfg = env_cfg;
    }
    if let Some(mode) = &cli.mode {
        cfg.mode = mode.parse::<wellfriendpdf_engine::ExecutionMode>()?;
    }
    cfg.validate()?;
    Ok(Some(serde_json::to_string(&cfg)?))
}

fn run_runtime_capabilities(
    args: RuntimeReportArgs,
    config_json: Option<&str>,
) -> Result<(), Box<dyn Error>> {
    let json = wellfriendpdf_engine::sdk::runtime_capabilities_json(config_json)?;
    write_output_optional(&args.output, &pretty_json(&json)?)
}

fn run_runtime_config(
    args: RuntimeConfigArgs,
    config_json: Option<&str>,
) -> Result<(), Box<dyn Error>> {
    let json = if args.validate {
        wellfriendpdf_engine::sdk::runtime_validate_config_json(config_json)?
    } else {
        wellfriendpdf_engine::sdk::runtime_effective_config_json(config_json)?
    };
    let _ = args.effective;
    write_output_optional(&args.output, &pretty_json(&json)?)
}

fn run_runtime_providers(
    args: ProvidersArgs,
    config_json: Option<&str>,
) -> Result<(), Box<dyn Error>> {
    let json = match args.command {
        ProviderCommand::List => wellfriendpdf_engine::sdk::ocr_provider_matrix_json()?,
        ProviderCommand::Check => {
            wellfriendpdf_engine::sdk::runtime_validate_config_json(config_json)?
        }
    };
    write_output_optional(&args.output, &pretty_json(&json)?)
}

fn run_extract_text(args: ExtractTextArgs) -> Result<(), Box<dyn Error>> {
    use rayon::prelude::*;

    let engine = match &args.password {
        Some(password) => wellfriendpdf_engine::ContentEngine::open_path_with_password(
            &args.pdf,
            password.as_bytes(),
        )?,
        None => wellfriendpdf_engine::ContentEngine::open_path(&args.pdf)?,
    };
    let total = engine.page_count()?;
    let page_nums = parse_page_range_cli(&args.pages, total)?;
    let region = args.region.as_deref().map(parse_region_cli).transpose()?;
    let profile = parse_profile_cli(&args.profile)?;

    if args.structured && args.semantic {
        return Err("--structured and --semantic are mutually exclusive".into());
    }
    let ocr_policy = ocr_policy_from_flag(&args.ocr)?;
    if ocr_policy.is_some() && (args.structured || args.semantic) {
        return Err("--ocr cannot be combined with --structured or --semantic".into());
    }
    if region.is_some() && args.semantic {
        return Err(
            "--region is not supported with --semantic; use default or --structured extraction"
                .into(),
        );
    }
    if region.is_some() && ocr_policy.is_some() {
        return Err("--region is not supported with --ocr text extraction yet".into());
    }

    // OCR path: route through the OCR-aware document parser so scanned
    // (image-only) pages contribute recovered text. Digital-born pages parse
    // exactly as before; only pages with no text layer change. Additive — the
    // default (no --ocr) path below is untouched.
    if let Some(policy) = ocr_policy {
        return run_extract_text_ocr(&engine, page_nums, &args, policy);
    }

    // Layout-aware and semantic extraction take separate additive paths and do
    // NOT change the default extraction below.
    if args.semantic {
        return run_extract_text_semantic(&engine, &page_nums, &args);
    }
    if args.structured {
        return run_extract_text_structured(&engine, &page_nums, &args);
    }

    // Per-page text rendering is independent and read-only. Process bounded
    // chunks across rayon workers so large files never put an unbounded number
    // of page buffers in flight. Each chunk preserves input order, and chunks
    // are appended sequentially, so the output stays byte-identical to serial
    // extraction.
    let parallel_window = wellfriendpdf_engine::bounded_text_parallel_window(page_nums.len());
    let page_texts: Vec<wellfriendpdf_engine::Result<String>> = if parallel_window >= 4 {
        let mut out = Vec::with_capacity(page_nums.len());
        for chunk in page_nums.chunks(parallel_window) {
            out.extend(
                chunk
                    .par_iter()
                    .map(|&page_num| match region {
                        Some(region) => engine.extract_text_in_region(page_num, region),
                        None => engine.get_page_text_with_profile(page_num, profile),
                    })
                    .collect::<Vec<_>>(),
            );
        }
        out
    } else {
        page_nums
            .iter()
            .map(|&page_num| match region {
                Some(region) => engine.extract_text_in_region(page_num, region),
                None => engine.get_page_text_with_profile(page_num, profile),
            })
            .collect()
    };

    let mut output_text = String::new();
    for (page_num, text) in page_nums.iter().zip(page_texts) {
        let text = text?;
        if args.page_numbers {
            output_text.push_str(&format!("--- Page {} ---\n", page_num));
        }
        output_text.push_str(&text);
        output_text.push('\n');
    }

    match &args.output {
        Some(path) => std::fs::write(path, output_text)?,
        None => print!("{}", output_text),
    }
    Ok(())
}

/// Layout-aware extraction: XY-cut segmentation recovers reading order across
/// columns/blocks. `--format text` emits reading-order text; `--format json`
/// emits the structured block tree (bounding boxes + reading order).
fn run_extract_text_structured(
    engine: &wellfriendpdf_engine::ContentEngine,
    page_nums: &[usize],
    args: &ExtractTextArgs,
) -> Result<(), Box<dyn Error>> {
    let region = args.region.as_deref().map(parse_region_cli).transpose()?;
    let format = args.format.to_lowercase();
    if matches!(
        format.as_str(),
        "model-json" | "model_json" | "semantic-model"
    ) {
        let model = engine.extract_text_semantic_model(page_nums, semantic_model_options(args)?)?;
        let s = serde_json::to_string_pretty(&model)?;
        match &args.output {
            Some(path) => std::fs::write(path, s)?,
            None => println!("{s}"),
        }
        return Ok(());
    }

    let as_json = match format.as_str() {
        "json" => true,
        "text" | "txt" => false,
        other => {
            return Err(usage_error(format!(
                "unknown --format '{other}'; use text, json, or model-json"
            )));
        }
    };

    if as_json {
        // One JSON object per document: pages -> blocks -> lines, in reading order.
        let mut pages = Vec::new();
        for &page_num in page_nums {
            let mut layout = engine.analyze_page_layout(page_num)?;
            if let Some(region) = region {
                for block in &mut layout.blocks {
                    block.lines.retain(|line| {
                        region.keeps_bbox([line.bbox.x0, line.bbox.y0, line.bbox.x1, line.bbox.y1])
                    });
                }
                layout.blocks.retain(|block| !block.lines.is_empty());
            }
            pages.push(serde_json::json!({
                "page": page_num,
                "blocks": layout.blocks,
            }));
        }
        let doc = serde_json::json!({ "pages": pages });
        let s = serde_json::to_string_pretty(&doc)?;
        match &args.output {
            Some(path) => std::fs::write(path, s)?,
            None => println!("{s}"),
        }
        return Ok(());
    }

    let mut out = String::new();
    for &page_num in page_nums {
        if args.page_numbers {
            out.push_str(&format!("--- Page {page_num} ---\n"));
        }
        match region {
            Some(region) => out.push_str(&engine.extract_text_in_region(page_num, region)?),
            None => out.push_str(&engine.get_page_text_structured(page_num)?),
        }
        out.push('\n');
    }
    match &args.output {
        Some(path) => std::fs::write(path, out)?,
        None => print!("{out}"),
    }
    Ok(())
}

/// Semantic extraction: tagged PDFs use `/StructTreeRoot` and MCID links;
/// untagged PDFs fall back to the geometric layout analyzer from `--structured`.
fn run_extract_text_semantic(
    engine: &wellfriendpdf_engine::ContentEngine,
    page_nums: &[usize],
    args: &ExtractTextArgs,
) -> Result<(), Box<dyn Error>> {
    let format = args.format.to_lowercase();
    if matches!(
        format.as_str(),
        "model-json" | "model_json" | "semantic-model"
    ) {
        let model = engine.extract_text_semantic_model(page_nums, semantic_model_options(args)?)?;
        let output = serde_json::to_string_pretty(&model)?;
        match &args.output {
            Some(path) => std::fs::write(path, output)?,
            None => println!("{output}"),
        }
        return Ok(());
    }

    let as_json = match format.as_str() {
        "json" => true,
        "text" | "txt" => false,
        other => {
            return Err(
                format!("unknown --format '{other}'; use text, json, or model-json").into(),
            );
        }
    };

    let document = engine.extract_semantic_document(page_nums)?;
    let output = if as_json {
        serde_json::to_string_pretty(&document)?
    } else {
        document.to_text()
    };

    match &args.output {
        Some(path) => std::fs::write(path, output)?,
        None => {
            if as_json {
                println!("{output}");
            } else {
                print!("{output}");
                if !output.ends_with('\n') {
                    println!();
                }
            }
        }
    }
    Ok(())
}

fn semantic_model_options(
    args: &ExtractTextArgs,
) -> Result<wellfriendpdf_engine::TextSemanticOptions, Box<dyn Error>> {
    let cjk_segmentation = match args.cjk_segmentation.to_ascii_lowercase().as_str() {
        "char" => wellfriendpdf_engine::CjkSegmentationMode::Char,
        "simple" => wellfriendpdf_engine::CjkSegmentationMode::Simple,
        "dictionary" | "dict" => wellfriendpdf_engine::CjkSegmentationMode::Dictionary,
        other => {
            return Err(usage_error(format!(
                "unknown --cjk-segmentation '{other}'; use char, simple, or dictionary"
            )));
        }
    };
    let defaults = wellfriendpdf_engine::TextSemanticOptions::default();
    Ok(wellfriendpdf_engine::TextSemanticOptions {
        include_structure: args.include_structure || args.semantic,
        include_detailed_provenance: args.include_provenance,
        cjk_segmentation,
        include_hidden: args.include_hidden || defaults.include_hidden,
        ..defaults
    })
}

/// OCR-aware text extraction: parse the document through the OCR seam so that
/// scanned (image-only) pages contribute recovered text, then emit the body
/// blocks as plain text in recovered reading order. Digital-born pages parse as
/// usual; only pages with no text layer differ from the non-OCR path.
fn run_extract_text_ocr(
    engine: &wellfriendpdf_engine::ContentEngine,
    page_nums: Vec<usize>,
    args: &ExtractTextArgs,
    policy: wellfriendpdf_engine::OcrPolicy,
) -> Result<(), Box<dyn Error>> {
    use wellfriendpdf_engine::{BlockKind, ParseOptions};

    let options = ParseOptions {
        pages: page_nums,
        // Keep furniture out of the text dump, matching the parser default.
        omit_furniture: true,
        ocr: Some(build_ocr_engine()?),
        ocr_policy: policy,
        ocr_options: ocr_options(&args.ocr_lang, args.ocr_dpi),
        ocr_dpi: args.ocr_dpi.max(1),
        ..ParseOptions::default()
    };
    let document = engine.parse_document(&options)?;

    // Walk the body blocks and emit their plain text, one logical block per
    // paragraph. Optional per-page markers mirror the non-OCR path.
    let mut out = String::new();
    let mut last_page: Option<u32> = None;
    for block in &document.body {
        if args.page_numbers && last_page != Some(block.page) {
            out.push_str(&format!("--- Page {} ---\n", block.page));
            last_page = Some(block.page);
        }
        let line = match &block.kind {
            BlockKind::Title { text }
            | BlockKind::Heading { text, .. }
            | BlockKind::Paragraph { text }
            | BlockKind::Caption { text, .. }
            | BlockKind::Header { text }
            | BlockKind::Footer { text }
            | BlockKind::PageNumber { text }
            | BlockKind::Text { text } => text.to_plain(),
            BlockKind::List { items, .. } => items
                .iter()
                .map(|it| it.text.to_plain())
                .collect::<Vec<_>>()
                .join("\n"),
            // Tables and figures carry no flowing prose; skip in a text dump.
            BlockKind::Table { .. } | BlockKind::Figure { .. } => continue,
        };
        if !line.trim().is_empty() {
            out.push_str(&line);
            out.push_str("\n\n");
        }
    }

    match &args.output {
        Some(path) => std::fs::write(path, out)?,
        None => print!("{out}"),
    }
    Ok(())
}

/// Detect and extract tables from a PDF — a capability Poppler's CLIs lack.
/// Ruled tables (drawn grid lines) and borderless tables (alignment-only) are
/// emitted as CSV (default), structured JSON, or span/header-preserving HTML.
fn run_extract_tables(args: ExtractTablesArgs) -> Result<(), Box<dyn Error>> {
    if args.ocr.is_some() {
        return Err(unsupported_error(
            "table extraction does not support --ocr: reconstructing a \
                    table grid from OCR'd word boxes is a known gap (see \
                    docs/parser_benchmark.md). For scanned documents, use \
                    `extract-fields --ocr` to recover key-value fields and line \
                    items, or `extract-text --ocr` for the recovered text.",
        ));
    }
    let _ = (&args.ocr_lang, args.ocr_dpi); // accepted for flag consistency only
    let engine = match &args.password {
        Some(password) => wellfriendpdf_engine::ContentEngine::open_path_with_password(
            &args.pdf,
            password.as_bytes(),
        )?,
        None => wellfriendpdf_engine::ContentEngine::open_path(&args.pdf)?,
    };
    let total = engine.page_count()?;
    let page_nums = parse_page_range_cli(&args.pages, total)?;
    let region = args.region.as_deref().map(parse_region_cli).transpose()?;

    let format = match args.format.to_lowercase().as_str() {
        "json" => "json",
        "html" | "htm" => "html",
        "csv" => "csv",
        other => return Err(format!("unknown --format '{other}'; use csv, json, or html").into()),
    };

    // (page, table) pairs above the confidence threshold, in page/reading order.
    let mut found = 0usize;
    let mut json_pages = Vec::new();
    let mut csv_out = String::new();
    let mut html_pages = Vec::new();

    for &page_num in &page_nums {
        let tables_raw = match region {
            Some(region) => engine.extract_tables_in_region(page_num, region)?,
            None => engine.extract_tables(page_num)?,
        };
        let tables: Vec<_> = tables_raw
            .into_iter()
            .filter(|t| t.confidence >= args.min_confidence)
            .collect();
        found += tables.len();

        match format {
            "json" => {
                json_pages.push(serde_json::json!({
                    "page": page_num,
                    "tables": tables,
                }));
            }
            "html" => {
                html_pages.push((page_num, tables));
            }
            _ => {
                for (i, t) in tables.iter().enumerate() {
                    if !csv_out.is_empty() {
                        csv_out.push('\n');
                    }
                    // A comment header makes multi-table CSV output navigable.
                    csv_out.push_str(&format!(
                        "# page {page_num} table {} ({:?}, confidence {:.2}, {}x{})\n",
                        i + 1,
                        t.source,
                        t.confidence,
                        t.num_rows(),
                        t.num_cols()
                    ));
                    csv_out.push_str(&t.to_csv());
                }
            }
        }
    }

    let output_text = match format {
        "json" => serde_json::to_string_pretty(&serde_json::json!({
            "structure": args.structure,
            "pages": json_pages
        }))?,
        "html" => table_pages_to_html(&html_pages),
        _ => csv_out,
    };

    match &args.output {
        Some(path) => std::fs::write(path, &output_text)?,
        None => print!("{output_text}"),
    }
    eprintln!(
        "Detected {found} table(s) across {} page(s)",
        page_nums.len()
    );
    Ok(())
}

fn table_pages_to_html(
    pages: &[(usize, Vec<wellfriendpdf_engine::analysis::tables::Table>)],
) -> String {
    let mut out = String::from(
        "<!doctype html>\n<html><head><meta charset=\"utf-8\"><title>Extracted Tables</title></head><body>\n",
    );
    for (page, tables) in pages {
        for (idx, table) in tables.iter().enumerate() {
            out.push_str(&format!(
                "<section data-page=\"{}\" data-table=\"{}\">\n",
                page,
                idx + 1
            ));
            out.push_str(&table.to_html());
            out.push_str("</section>\n");
        }
    }
    out.push_str("</body></html>\n");
    out
}

/// Construct the OCR backend for `--ocr`. Behind the `ocr` cargo feature: with
/// the feature on, this discovers and probes the external `tesseract` binary;
/// with the feature off, it returns an actionable error so a default
/// (pure-Rust) CLI build still parses, just without OCR.
#[cfg(feature = "ocr")]
fn build_ocr_engine() -> Result<std::sync::Arc<dyn wellfriendpdf_engine::OcrEngine>, Box<dyn Error>>
{
    let engine = wellfriendpdf_ocr_tesseract::TesseractEngine::new()?;
    Ok(std::sync::Arc::new(engine))
}

#[cfg(not(feature = "ocr"))]
fn build_ocr_engine() -> Result<std::sync::Arc<dyn wellfriendpdf_engine::OcrEngine>, Box<dyn Error>>
{
    Err(unsupported_error(
        "this build of wellfriendpdf has no OCR backend; rebuild the CLI with \
         `--features ocr` (and install the `tesseract` binary + language data) \
         to use --ocr",
    ))
}

/// Build [`wellfriendpdf_engine::OcrOptions`] from the shared CLI `--ocr-lang`/`--ocr-dpi`
/// flags (languages split on `+`/`,`, falling back to `eng`). Used by every
/// command that supports `--ocr` so the option parsing stays identical.
fn ocr_options(ocr_lang: &str, ocr_dpi: u32) -> wellfriendpdf_engine::OcrOptions {
    let langs: Vec<String> = ocr_lang
        .split(['+', ','])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    wellfriendpdf_engine::OcrOptions {
        languages: if langs.is_empty() {
            vec!["eng".to_string()]
        } else {
            langs
        },
        dpi: ocr_dpi,
        psm: None,
    }
}

/// Interpret the CLI `--ocr` flag value into an [`wellfriendpdf_engine::OcrPolicy`].
///
/// `--ocr` is an optional-value flag: absent → `None` (OCR off, the default);
/// bare `--ocr` → `Some("auto")` via clap's `default_missing_value`; `--ocr off`
/// / `auto` / `force` map to the matching policy. Returns `Ok(None)` when OCR is
/// off (the caller skips the OCR path), `Ok(Some(policy))` otherwise, or an
/// error for an unrecognized token.
fn ocr_policy_from_flag(
    flag: &Option<String>,
) -> Result<Option<wellfriendpdf_engine::OcrPolicy>, Box<dyn Error>> {
    match flag.as_deref() {
        None => Ok(None),
        Some(tok) => match wellfriendpdf_engine::OcrPolicy::parse(tok) {
            Some(wellfriendpdf_engine::OcrPolicy::Off) => Ok(None),
            Some(policy) => Ok(Some(policy)),
            None => Err(format!(
                "invalid --ocr value '{tok}'; use off, auto, or force (bare --ocr means auto)"
            )
            .into()),
        },
    }
}

/// Build a typed, ordered document model — a real document outline (headings,
/// paragraphs, lists, figures, captions, tables in reading order), not a text
/// dump. Tagged PDFs use their authored structure; untagged PDFs use the
/// geometric precedence-graph ordering + semantic classifier. JSON emits the
/// full model; markdown emits a readable rendering for human inspection.
fn run_parse(args: ParseArgs) -> Result<(), Box<dyn Error>> {
    use wellfriendpdf_engine::{ImageHandling, ParseOptions, SerializeOptions};

    #[derive(Clone, Copy)]
    enum Fmt {
        Markdown,
        Json,
        Html,
    }
    let fmt = match args.format.to_lowercase().as_str() {
        "markdown" | "md" | "text" | "txt" => Fmt::Markdown,
        "json" => Fmt::Json,
        "html" => Fmt::Html,
        other => {
            return Err(format!("unknown --format '{other}'; use markdown, json, or html").into());
        }
    };

    let engine = open_engine(&args.pdf, &args.password)?;
    let total = engine.page_count()?;
    let page_nums = parse_page_range_cli(&args.pages, total)?;
    let profile = parse_profile_cli(&args.profile)?;

    let mut options = ParseOptions {
        pages: page_nums,
        min_confidence: args.min_confidence,
        omit_furniture: !args.keep_furniture,
        images: match &args.images_dir {
            Some(dir) => ImageHandling::SidecarDir(dir.clone()),
            None => ImageHandling::Omit,
        },
        dehyphenate: args.dehyphenate,
        normalize_ligatures: args.normalize_ligatures,
        ..ParseOptions::default()
    };
    if let Some(policy) = ocr_policy_from_flag(&args.ocr)? {
        options.ocr = Some(build_ocr_engine()?);
        options.ocr_policy = policy;
        options.ocr_options = ocr_options(&args.ocr_lang, args.ocr_dpi);
        options.ocr_dpi = args.ocr_dpi.max(1);
    }
    let output_pages = options.pages.clone();
    let document = engine.parse_document_with_profile(profile, &options)?;

    let ser_opts = SerializeOptions {
        include_furniture: args.keep_furniture,
        mark_page_breaks: args.mark_page_breaks,
        include_provenance: args.provenance,
    };
    let output_text = match fmt {
        Fmt::Json => document.to_json(),
        Fmt::Markdown if args.detect_headings => document.to_markdown(&ser_opts),
        Fmt::Markdown => engine.to_markdown_with_options(&output_pages, false, &ser_opts)?,
        Fmt::Html => document.to_html(&ser_opts),
    };

    match &args.output {
        Some(path) => std::fs::write(path, &output_text)?,
        None => {
            print!("{output_text}");
            if !output_text.ends_with('\n') {
                println!();
            }
        }
    }
    let scanned = document
        .pages
        .iter()
        .filter(|p| p.source == wellfriendpdf_engine::PageSource::Scanned)
        .count();
    eprintln!(
        "Parsed: {} body block(s) across {} page(s) ({:?} source, schema {}){}",
        document.body.len(),
        document.pages.len(),
        document.source,
        document.schema_version,
        if scanned > 0 && options.ocr.is_none() {
            format!("; {scanned} scanned page(s) routed to OCR (no engine; placeholder)")
        } else if scanned > 0 {
            format!("; {scanned} scanned page(s) OCR'd")
        } else {
            String::new()
        },
    );
    Ok(())
}

/// Extract structured key-value fields to JSON (the data-automation surface).
fn run_extract_fields(args: ExtractFieldsArgs) -> Result<(), Box<dyn Error>> {
    use wellfriendpdf_engine::{DocType, ExtractOptions};

    if !matches!(args.format.to_lowercase().as_str(), "json") {
        return Err(format!("unknown --format '{}'; only json is supported", args.format).into());
    }

    let doc_type = match args.r#type.to_lowercase().as_str() {
        "auto" => None,
        other => Some(DocType::parse(other).ok_or_else(|| {
            format!("unknown --type '{other}'; use auto, invoice, receipt, form, or generic")
        })?),
    };

    let engine = open_engine(&args.pdf, &args.password)?;
    let total = engine.page_count()?;
    let page_nums = parse_page_range_cli(&args.pages, total)?;

    let mut options = ExtractOptions {
        doc_type,
        pages: page_nums,
        min_confidence: args.min_confidence,
        ..Default::default()
    };
    if let Some(policy) = ocr_policy_from_flag(&args.ocr)? {
        options.ocr = Some(build_ocr_engine()?);
        options.ocr_policy = policy;
        options.ocr_options = ocr_options(&args.ocr_lang, args.ocr_dpi);
        options.ocr_dpi = args.ocr_dpi.max(1);
    }

    let result = engine.extract_fields(&options)?;
    let output_text = result.to_json();

    match &args.output {
        Some(path) => std::fs::write(path, &output_text)?,
        None => {
            print!("{output_text}");
            if !output_text.ends_with('\n') {
                println!();
            }
        }
    }
    let low = result.fields.iter().filter(|f| f.confidence < 0.5).count();
    eprintln!(
        "Extracted {} field(s){} from a {:?}{} document ({} line item(s)).",
        result.fields.len(),
        if low > 0 {
            format!(" ({low} low-confidence)")
        } else {
            String::new()
        },
        result.doc_type,
        if result.doc_type_forced {
            " (forced)"
        } else {
            " (auto-detected)"
        },
        result.line_items.len(),
    );
    Ok(())
}

fn run_forms_report(args: FormsReportArgs) -> Result<(), Box<dyn Error>> {
    let engine = open_engine(&args.pdf, &args.password)?;
    let output = serde_json::to_string_pretty(&wellfriendpdf_engine::forms_report(&engine)?)?;
    write_output_optional(&args.output, &output)?;
    Ok(())
}

fn run_xfa_report(args: XfaReportArgs) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let output = wellfriendpdf_engine::sdk::xfa_report_json(
        &bytes,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&output)?)?;
    Ok(())
}

fn run_xfa_extract(args: XfaReportArgs) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let output = wellfriendpdf_engine::sdk::xfa_extract_json(
        &bytes,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&output)?)?;
    Ok(())
}

fn run_xfa_script_report(args: XfaReportArgs) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let output = wellfriendpdf_engine::sdk::xfa_script_report_json(
        &bytes,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&output)?)?;
    Ok(())
}

fn run_xfa_security_report(args: XfaReportArgs) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let output = wellfriendpdf_engine::sdk::xfa_security_report_json(
        &bytes,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&output)?)?;
    Ok(())
}

fn run_xfa_runtime_report(args: XfaRuntimeReportArgs) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let output = wellfriendpdf_engine::sdk::xfa_runtime_report_json(
        &bytes,
        Some(&args.script_policy),
        args.execute_events,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&output)?)?;
    Ok(())
}

fn run_xfa_render(args: XfaRenderArgs) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let (output, report) = wellfriendpdf_engine::sdk::xfa_render_preview_json(
        &bytes,
        Some(&args.script_policy),
        args.execute_events,
        args.dpi,
        args.password.as_deref().map(str::as_bytes),
    )?;
    std::fs::write(&args.output, &output)?;
    write_xfa_operation_report(&report, args.report.as_ref(), args.json)?;
    if !args.json {
        eprintln!("Rendered XFA preview -> {}", args.output.display());
    }
    Ok(())
}

fn run_xfa_flatten(args: XfaFlattenArgs) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let (output, report) = wellfriendpdf_engine::sdk::xfa_flatten_json(
        &bytes,
        Some(&args.mode),
        args.password.as_deref().map(str::as_bytes),
    )?;
    std::fs::write(&args.output, &output)?;
    write_xfa_operation_report(&report, args.report.as_ref(), args.json)?;
    if !args.json {
        eprintln!(
            "Flattened supported XFA subset -> {}",
            args.output.display()
        );
    }
    Ok(())
}

fn run_xfa_sanitize(args: XfaSanitizeArgs) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let (output, report) = wellfriendpdf_engine::sdk::xfa_sanitize_json(
        &bytes,
        Some(&args.mode),
        args.password.as_deref().map(str::as_bytes),
    )?;
    std::fs::write(&args.output, &output)?;
    write_xfa_operation_report(&report, args.report.as_ref(), args.json)?;
    if !args.json {
        eprintln!("Sanitized XFA -> {}", args.output.display());
    }
    Ok(())
}

fn pretty_json(json: &str) -> Result<String, Box<dyn Error>> {
    let value: serde_json::Value = serde_json::from_str(json)?;
    Ok(serde_json::to_string_pretty(&value)?)
}

fn write_xfa_operation_report(
    report: &str,
    output: Option<&PathBuf>,
    print_json: bool,
) -> Result<(), Box<dyn Error>> {
    let pretty = pretty_json(report)?;
    if let Some(path) = output {
        std::fs::write(path, &pretty)?;
    }
    if print_json {
        println!("{pretty}");
    }
    Ok(())
}

fn run_annotation_xfdf_export(args: AnnotationXfdfExportArgs) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let (xfdf, report) = wellfriendpdf_engine::sdk::annotation_xfdf_export_json(
        &bytes,
        args.password.as_deref().map(str::as_bytes),
    )?;
    std::fs::write(&args.output, xfdf)?;
    write_xfa_operation_report(&report, args.report.as_ref(), args.json)?;
    if !args.json {
        eprintln!("Exported annotation XFDF -> {}", args.output.display());
    }
    Ok(())
}

fn run_annotation_xfdf_import(args: AnnotationXfdfImportArgs) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let xfdf = std::fs::read(&args.xfdf)?;
    let options = args
        .options
        .as_ref()
        .map(std::fs::read_to_string)
        .transpose()?;
    let (output, report) = wellfriendpdf_engine::sdk::annotation_xfdf_import_json(
        &bytes,
        &xfdf,
        options.as_deref(),
        args.password.as_deref().map(str::as_bytes),
    )?;
    if !args.dry_run {
        std::fs::write(&args.output, output)?;
    }
    write_xfa_operation_report(&report, args.report.as_ref(), args.json)?;
    if !args.json && !args.dry_run {
        eprintln!("Imported annotation XFDF -> {}", args.output.display());
    }
    Ok(())
}

fn run_annotation_appearance_generate(
    args: AnnotationAppearanceGenerateArgs,
) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let options = args
        .options
        .as_ref()
        .map(std::fs::read_to_string)
        .transpose()?;
    let (output, report) = wellfriendpdf_engine::sdk::annotation_appearance_generate_json(
        &bytes,
        options.as_deref(),
        args.password.as_deref().map(str::as_bytes),
    )?;
    if !args.dry_run {
        std::fs::write(&args.output, output)?;
    }
    write_xfa_operation_report(&report, args.report.as_ref(), args.json)?;
    if !args.json && !args.dry_run {
        eprintln!(
            "Generated annotation appearances -> {}",
            args.output.display()
        );
    }
    Ok(())
}

fn run_annotation_appearance_report(
    args: AnnotationAppearanceReportArgs,
) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let options = args
        .options
        .as_ref()
        .map(std::fs::read_to_string)
        .transpose()?;
    let report = wellfriendpdf_engine::sdk::annotation_appearance_report_json(
        &bytes,
        options.as_deref(),
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)?;
    Ok(())
}

fn run_rich_media_report(args: AnnotationMediaRedactionReportArgs) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let report = wellfriendpdf_engine::sdk::rich_media_report_json(
        &bytes,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)?;
    Ok(())
}

fn run_rich_media_sanitize(args: RichMediaSanitizeArgs) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let custom = args
        .custom
        .as_ref()
        .map(std::fs::read_to_string)
        .transpose()?;
    let (output, report) = wellfriendpdf_engine::sdk::rich_media_sanitize_json(
        &bytes,
        Some(&args.policy),
        custom.as_deref(),
        args.password.as_deref().map(str::as_bytes),
    )?;
    if !args.dry_run {
        std::fs::write(&args.output, output)?;
    }
    write_xfa_operation_report(&report, args.report.as_ref(), args.json)?;
    if !args.json && !args.dry_run {
        eprintln!("Applied rich-media policy -> {}", args.output.display());
    }
    Ok(())
}

fn run_rich_media_flatten_poster(
    args: AnnotationMediaRedactionOutputArgs,
) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let (output, report) = wellfriendpdf_engine::sdk::rich_media_flatten_poster_json(
        &bytes,
        args.password.as_deref().map(str::as_bytes),
    )?;
    if !args.dry_run {
        std::fs::write(&args.output, output)?;
    }
    write_xfa_operation_report(&report, args.report.as_ref(), args.json)?;
    if !args.json && !args.dry_run {
        eprintln!(
            "Flattened static media posters -> {}",
            args.output.display()
        );
    }
    Ok(())
}

fn run_nonaxis_redaction(args: NonAxisRedactionArgs) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let options = std::fs::read_to_string(&args.plan)?;
    if args.dry_run {
        let report = wellfriendpdf_engine::sdk::nonaxis_redaction_plan_json(
            &bytes,
            &options,
            args.password.as_deref().map(str::as_bytes),
        )?;
        write_xfa_operation_report(&report, args.report.as_ref(), true)?;
        return Ok(());
    }
    let (output, report) = wellfriendpdf_engine::sdk::nonaxis_redaction_apply_json(
        &bytes,
        &options,
        args.password.as_deref().map(str::as_bytes),
    )?;
    std::fs::write(&args.output, output)?;
    write_xfa_operation_report(&report, args.report.as_ref(), args.json)?;
    if !args.json {
        eprintln!(
            "Applied non-axis image redaction -> {}",
            args.output.display()
        );
    }
    Ok(())
}

fn run_annotation_media_redaction_report(
    args: AnnotationMediaRedactionReportArgs,
) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let report = wellfriendpdf_engine::sdk::annotation_media_redaction_report_json(
        &bytes,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)?;
    Ok(())
}

fn run_secure_mutation_report(
    args: AnnotationMediaRedactionReportArgs,
) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let report = wellfriendpdf_engine::sdk::secure_mutation_report_json(
        &bytes,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_secure_mutation_closeout_report(
    args: AnnotationMediaRedactionReportArgs,
) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let report = wellfriendpdf_engine::sdk::secure_mutation_closeout_report_json(
        &bytes,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_form_js_report(args: AnnotationMediaRedactionReportArgs) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let report = wellfriendpdf_engine::sdk::form_js_report_json(
        &bytes,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_form_js_sanitize(
    args: FormActionPolicySanitizeArgs,
    flatten: bool,
) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let options = if let Some(path) = &args.options {
        std::fs::read_to_string(path)?
    } else {
        let mode = if flatten {
            "flatten_calculated_values_then_remove"
        } else {
            wellfriendpdf_engine::FormJsPolicyMode::parse(&args.policy)
                .ok_or_else(|| usage_error("unknown --policy for form-js-sanitize"))?
                .as_str()
        };
        serde_json::to_string(&serde_json::json!({
            "mode": mode,
            "signature_policy_override": args.signature_policy_override,
            "limits": wellfriendpdf_engine::FormJsLimits::default()
        }))?
    };
    if args.dry_run {
        let report = if flatten {
            wellfriendpdf_engine::sdk::form_action_graph_json(
                &bytes,
                args.password.as_deref().map(str::as_bytes),
            )?
        } else {
            wellfriendpdf_engine::sdk::form_js_report_json(
                &bytes,
                args.password.as_deref().map(str::as_bytes),
            )?
        };
        return write_xfa_operation_report(&report, args.report.as_ref(), true);
    }
    let (output, report) = if flatten {
        wellfriendpdf_engine::sdk::form_js_flatten_values_json(
            &bytes,
            Some(&options),
            args.password.as_deref().map(str::as_bytes),
        )?
    } else {
        wellfriendpdf_engine::sdk::form_js_sanitize_json(
            &bytes,
            Some(&options),
            args.password.as_deref().map(str::as_bytes),
        )?
    };
    std::fs::write(&args.output, output)?;
    write_xfa_operation_report(&report, args.report.as_ref(), args.json)?;
    if !args.json {
        eprintln!(
            "Wrote form action policy action-policy output -> {}",
            args.output.display()
        );
    }
    Ok(())
}

fn run_interactive_data_closeout_report(
    args: AnnotationMediaRedactionReportArgs,
) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let report = wellfriendpdf_engine::sdk::interactive_data_closeout_report_json(
        &bytes,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_word_pagination_audit(args: WordPaginationAuditArgs) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let raw = wellfriendpdf_engine::sdk::word_pagination_audit_json(
        &bytes,
        &args.layout,
        args.password.as_deref().map(str::as_bytes),
    )?;
    let mut report: serde_json::Value = serde_json::from_str(&raw)?;
    report["external_harness_request"] = serde_json::json!({
        "word": args.word.as_ref().map(|path| serde_json::json!({
            "path": path.display().to_string(),
            "exists": path.exists(),
            "status": "configured_for_external_automation; not_inferred_from_ooxml"
        })).unwrap_or_else(|| serde_json::json!({"status": "not_requested"})),
        "libreoffice": args.libreoffice.as_ref().map(|path| serde_json::json!({
            "path": path.display().to_string(),
            "exists": path.exists(),
            "status": "configured_for_external_export; form_action_policy audit harness owns execution"
        })).unwrap_or_else(|| serde_json::json!({"status": "not_requested"}))
    });
    if args.fail_on_unsupported
        && report["report"]["unsupported_exact"]
            .as_array()
            .is_some_and(|rows| !rows.is_empty())
    {
        return Err(usage_error(
            "word-pagination-audit found exact unsupported rows",
        ));
    }
    let pretty = serde_json::to_string_pretty(&report)?;
    write_output_optional(&args.output, &pretty)
}

fn run_form_action_policy_report(
    args: AnnotationMediaRedactionReportArgs,
) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let report = wellfriendpdf_engine::sdk::form_action_policy_report_json(
        &bytes,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_advanced_editing_report(
    args: AnnotationMediaRedactionReportArgs,
) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let report = wellfriendpdf_engine::sdk::advanced_editing_report_json(
        &bytes,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_advanced_editing_closeout_report(
    args: AnnotationMediaRedactionReportArgs,
) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let report = wellfriendpdf_engine::sdk::advanced_editing_closeout_report_json(
        &bytes,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_writer_history_report(
    args: AnnotationMediaRedactionReportArgs,
) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let report = wellfriendpdf_engine::sdk::writer_history_report_json(
        &bytes,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_writer_history_raster_vector_report(
    args: WriterHistoryRasterVectorArgs,
) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let options_json = match args.options {
        Some(path) => Some(std::fs::read_to_string(path)?),
        None => None,
    };
    let report = wellfriendpdf_engine::sdk::writer_history_raster_vector_report_json(
        &bytes,
        args.page,
        options_json.as_deref(),
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_writer_history_font_reconstruction_report(
    args: AnnotationMediaRedactionReportArgs,
) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let report = wellfriendpdf_engine::sdk::writer_history_font_reconstruction_report_json(
        &bytes,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_writer_history_history_report(args: WriterHistoryHistoryArgs) -> Result<(), Box<dyn Error>> {
    let report = wellfriendpdf_engine::sdk::writer_history_history_report_json()?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_writer_history_object_stream_report(
    args: AnnotationMediaRedactionReportArgs,
) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let report = wellfriendpdf_engine::sdk::writer_history_object_stream_report_json(
        &bytes,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_writer_history_save_object_streams(
    args: WriterHistorySaveObjectStreamsArgs,
) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let (output, report) = wellfriendpdf_engine::sdk::writer_history_pack_object_streams_json(
        &bytes,
        args.password.as_deref().map(str::as_bytes),
    )?;
    if !args.dry_run {
        std::fs::write(&args.output, output)?;
    }
    let pretty = pretty_json(&report)?;
    if let Some(path) = args.report {
        std::fs::write(path, &pretty)?;
    }
    if args.json || !args.dry_run {
        println!("{pretty}");
    }
    Ok(())
}

fn run_compression_office_report(
    args: AnnotationMediaRedactionReportArgs,
) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let report = wellfriendpdf_engine::sdk::compression_office_report_json(
        &bytes,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_crypto_writer_report(
    args: AnnotationMediaRedactionReportArgs,
) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let report = wellfriendpdf_engine::sdk::crypto_writer_report_json(
        &bytes,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_writer_determinism_audit(
    args: AnnotationMediaRedactionReportArgs,
) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let report = wellfriendpdf_engine::sdk::writer_determinism_audit_json(
        &bytes,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_writer_external_diff(
    args: AnnotationMediaRedactionReportArgs,
) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let report = wellfriendpdf_engine::sdk::writer_external_diff_json(
        &bytes,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_writer_closeout_report(
    args: AnnotationMediaRedactionReportArgs,
) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let report = wellfriendpdf_engine::sdk::writer_closeout_report_json(
        &bytes,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_pubsec_report(args: AnnotationMediaRedactionReportArgs) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let report = wellfriendpdf_engine::sdk::pubsec_report_json(
        &bytes,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_pubsec_decrypt(args: CryptoWriterCryptoReportArgs) -> Result<(), Box<dyn Error>> {
    let output = args
        .pdf_output
        .as_ref()
        .ok_or("pubsec-decrypt requires --pdf-output to write decrypted PDF bytes")?;
    refuse_overwrite(output)?;
    let provider = load_pubsec_provider(&args)?;
    let bytes = std::fs::read(&args.pdf)?;
    let engine =
        wellfriendpdf_engine::ContentEngine::open_bytes_with_pubsec_provider(bytes, &provider)?;
    let out = wellfriendpdf_engine::decrypt_pdf(&engine)?;
    std::fs::write(output, &out)?;
    let report = serde_json::json!({
        "operation": "pubsec_decrypt",
        "status": "implemented_with_limits",
        "output_pdf": output.display().to_string(),
        "output_pdf_written": true,
        "bytes": out.len(),
        "certificate_path_count": args.certificate.len(),
        "private_key_path_count": args.private_key.len(),
        "pfx_provider_configured": args.pfx.is_some(),
        "secret_material_reported": false,
    });
    write_output_optional(&args.output, &serde_json::to_string_pretty(&report)?)
}

fn run_pubsec_encrypt(args: CryptoWriterCryptoReportArgs) -> Result<(), Box<dyn Error>> {
    let output = args
        .pdf_output
        .as_ref()
        .ok_or("pubsec-encrypt requires --pdf-output to write encrypted PDF bytes")?;
    refuse_overwrite(output)?;
    let engine = open_engine(&args.pdf, &args.password)?;
    let options = pubsec_encrypt_options_from_args(&args)?;
    let (out, pubsec_report) =
        wellfriendpdf_engine::encrypt_pdf_pubsec(engine.document().reader(), &options)?;
    std::fs::write(output, &out)?;
    let report = serde_json::json!({
        "operation": "pubsec_encrypt",
        "status": "implemented_with_limits",
        "output_pdf": output.display().to_string(),
        "output_pdf_written": true,
        "bytes": out.len(),
        "recipient_count": pubsec_report.recipient_count,
        "crypt_filter": pubsec_report.crypt_filter,
        "method": format!("{:?}", pubsec_report.method),
        "permissions": pubsec_report.permissions,
        "encrypt_metadata": pubsec_report.encrypt_metadata,
        "secret_material_reported": false,
    });
    write_output_optional(&args.output, &serde_json::to_string_pretty(&report)?)
}

fn run_pubsec_reencrypt(args: CryptoWriterCryptoReportArgs) -> Result<(), Box<dyn Error>> {
    run_pubsec_reencrypt_with_operation(args, "pubsec_reencrypt")
}

fn run_pubsec_reencrypt_with_operation(
    args: CryptoWriterCryptoReportArgs,
    operation: &str,
) -> Result<(), Box<dyn Error>> {
    let output = args
        .pdf_output
        .as_ref()
        .ok_or("PubSec recipient mutation requires --pdf-output to write encrypted PDF bytes")?;
    refuse_overwrite(output)?;
    let options = pubsec_encrypt_options_from_args(&args)?;
    let engine = if args.private_key.is_empty() && args.pfx.is_none() {
        open_engine(&args.pdf, &args.password)?
    } else {
        let provider = load_pubsec_provider(&args)?;
        let bytes = std::fs::read(&args.pdf)?;
        wellfriendpdf_engine::ContentEngine::open_bytes_with_pubsec_provider(bytes, &provider)?
    };
    let (out, pubsec_report) =
        wellfriendpdf_engine::reencrypt_pdf_pubsec(engine.document().reader(), &options)?;
    std::fs::write(output, &out)?;
    let report = serde_json::json!({
        "operation": operation,
        "status": "implemented_with_limits",
        "recipient_mutation_policy": "full_rewrite_rotates_file_key",
        "output_pdf": output.display().to_string(),
        "output_pdf_written": true,
        "bytes": out.len(),
        "recipient_count": pubsec_report.recipient_count,
        "old_provider_supplied": !args.private_key.is_empty() || args.pfx.is_some(),
        "secret_material_reported": false,
    });
    write_output_optional(&args.output, &serde_json::to_string_pretty(&report)?)
}

fn run_pdf_mac_report(args: AnnotationMediaRedactionReportArgs) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let report = wellfriendpdf_engine::sdk::pdf_mac_report_json(
        &bytes,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_pdf_mac_verify(args: AnnotationMediaRedactionReportArgs) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let report = wellfriendpdf_engine::sdk::pdf_mac_verify_json(
        &bytes,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_pdf_mac_create(args: CryptoWriterCryptoReportArgs) -> Result<(), Box<dyn Error>> {
    let output = args
        .pdf_output
        .as_ref()
        .ok_or("pdf-mac-create requires --pdf-output to write protected PDF bytes")?;
    refuse_overwrite(output)?;
    let bytes = std::fs::read(&args.pdf)?;
    let (out, report) = wellfriendpdf_engine::sdk::pdf_mac_create_json(
        &bytes,
        args.password.as_deref().map(str::as_bytes),
    )?;
    std::fs::write(output, &out)?;
    let mut value: serde_json::Value = serde_json::from_str(&report)?;
    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "operation".to_string(),
            serde_json::Value::String("pdf_mac_create".to_string()),
        );
        obj.insert(
            "output_pdf".to_string(),
            serde_json::Value::String(output.display().to_string()),
        );
        obj.insert(
            "output_pdf_written".to_string(),
            serde_json::Value::Bool(true),
        );
    }
    write_output_optional(&args.output, &serde_json::to_string_pretty(&value)?)
}

fn load_pubsec_provider(
    args: &CryptoWriterCryptoReportArgs,
) -> Result<wellfriendpdf_engine::PubSecKeyProvider, Box<dyn Error>> {
    if let Some(pfx_path) = &args.pfx {
        if !args.certificate.is_empty() || !args.private_key.is_empty() {
            return Err(
                "PubSec provider accepts either --pfx or --certificate/--private-key, not both"
                    .into(),
            );
        }
        let pfx = std::fs::read(pfx_path)?;
        let password = read_private_key_password(args)?;
        let identity = wellfriendpdf_engine::PubSecIdentity::from_pkcs12_der(&pfx, &password)?;
        return Ok(wellfriendpdf_engine::PubSecKeyProvider::single(identity));
    }
    if args.certificate.len() != 1 || args.private_key.len() != 1 {
        return Err(
            "PubSec provider operations require exactly one --certificate and one --private-key"
                .into(),
        );
    }
    let cert = std::fs::read(&args.certificate[0])?;
    let key = std::fs::read(&args.private_key[0])?;
    let password = read_private_key_password(args)?;
    let identity = if password.is_empty() {
        wellfriendpdf_engine::PubSecIdentity::from_bytes(&cert, &key)?
    } else {
        wellfriendpdf_engine::PubSecIdentity::from_encrypted_pkcs8_der(&cert, &key, &password)?
    };
    Ok(wellfriendpdf_engine::PubSecKeyProvider::single(identity))
}

fn read_private_key_password(
    args: &CryptoWriterCryptoReportArgs,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let Some(path) = &args.private_key_password_file else {
        return Ok(Vec::new());
    };
    let mut bytes = std::fs::read(path)?;
    while matches!(bytes.last(), Some(b'\r' | b'\n')) {
        bytes.pop();
    }
    Ok(bytes)
}

fn pubsec_encrypt_options_from_args(
    args: &CryptoWriterCryptoReportArgs,
) -> Result<wellfriendpdf_engine::PubSecEncryptOptions, Box<dyn Error>> {
    let recipient_paths = if args.recipient_certificate.is_empty() {
        &args.certificate
    } else {
        &args.recipient_certificate
    };
    if recipient_paths.is_empty() {
        return Err(
            "PubSec encrypt/re-encrypt requires at least one --recipient-certificate or --certificate"
                .into(),
        );
    }
    let mut recipients = Vec::with_capacity(recipient_paths.len());
    for path in recipient_paths {
        let bytes = std::fs::read(path)?;
        recipients.push(wellfriendpdf_engine::PubSecRecipientCertificate::from_bytes(&bytes)?);
    }
    Ok(wellfriendpdf_engine::PubSecEncryptOptions {
        recipients,
        permissions: args.permissions as u32,
        encrypt_metadata: true,
        method: wellfriendpdf_engine::CryptMethod::AesV2,
        recipient_id_mode: wellfriendpdf_engine::PubSecRecipientIdMode::IssuerAndSerial,
    })
}

fn refuse_overwrite(path: &std::path::Path) -> Result<(), Box<dyn Error>> {
    if path.exists() {
        return Err(format!("refusing to overwrite existing output {}", path.display()).into());
    }
    Ok(())
}

fn run_aes_gcm_report(args: AnnotationMediaRedactionReportArgs) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let report = wellfriendpdf_engine::sdk::aes_gcm_report_json(
        &bytes,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_aes_gcm_encrypt(args: CryptoWriterCryptoReportArgs) -> Result<(), Box<dyn Error>> {
    use wellfriendpdf_engine::crypto::{secret_bytes, EncryptAlgorithm, EncryptParams};

    let output = args
        .pdf_output
        .as_ref()
        .ok_or("aes-gcm-encrypt requires --pdf-output to write encrypted PDF bytes")?;
    if output.exists() {
        return Err(format!("refusing to overwrite existing output {}", output.display()).into());
    }
    let engine = open_engine(&args.pdf, &args.password)?;
    let user_pw = args
        .user_pw
        .clone()
        .or_else(|| args.password.clone())
        .unwrap_or_default();
    let owner_pw = args.owner_pw.clone().unwrap_or_else(|| user_pw.clone());
    let params = EncryptParams {
        user_password: secret_bytes(user_pw.into_bytes()),
        owner_password: secret_bytes(owner_pw.into_bytes()),
        permissions: args.permissions,
        algorithm: EncryptAlgorithm::Aes256Gcm,
        encrypt_metadata: true,
    };
    let bytes = wellfriendpdf_engine::encrypt(&engine, &params)?;
    std::fs::write(output, &bytes)?;
    let report = serde_json::json!({
        "operation": "aes_gcm_encrypt",
        "status": "implemented_with_limits",
        "output_pdf": output.display().to_string(),
        "output_pdf_written": true,
        "bytes": bytes.len(),
        "algorithm": "aes256gcm",
        "certificate_path_configured": !args.certificate.is_empty(),
        "private_key_path_configured": !args.private_key.is_empty(),
        "secret_material_reported": false,
    });
    write_output_optional(&args.output, &serde_json::to_string_pretty(&report)?)
}

fn run_aes_gcm_decrypt(args: CryptoWriterCryptoReportArgs) -> Result<(), Box<dyn Error>> {
    let output = args
        .pdf_output
        .as_ref()
        .ok_or("aes-gcm-decrypt requires --pdf-output to write decrypted PDF bytes")?;
    if output.exists() {
        return Err(format!("refusing to overwrite existing output {}", output.display()).into());
    }
    let engine = open_engine(&args.pdf, &args.password)?;
    let bytes = wellfriendpdf_engine::decrypt_pdf(&engine)?;
    std::fs::write(output, &bytes)?;
    let report = serde_json::json!({
        "operation": "aes_gcm_decrypt",
        "status": "implemented_with_limits",
        "output_pdf": output.display().to_string(),
        "output_pdf_written": true,
        "bytes": bytes.len(),
        "certificate_path_configured": !args.certificate.is_empty(),
        "private_key_path_configured": !args.private_key.is_empty(),
        "secret_material_reported": false,
    });
    write_output_optional(&args.output, &serde_json::to_string_pretty(&report)?)
}

fn run_crypto_tamper_test(args: CryptoWriterTamperArgs) -> Result<(), Box<dyn Error>> {
    let report = wellfriendpdf_engine::sdk::crypto_tamper_test_json()?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_compression_office_optimize(
    args: CompressionOfficeOptimizeArgs,
) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let options_json = match args.options {
        Some(path) => Some(std::fs::read_to_string(path)?),
        None => None,
    };
    let (output, report) = wellfriendpdf_engine::sdk::compression_office_optimize_pdf_json(
        &bytes,
        options_json.as_deref(),
        args.password.as_deref().map(str::as_bytes),
    )?;
    if !args.dry_run {
        std::fs::write(&args.output, output)?;
    }
    let pretty = pretty_json(&report)?;
    if let Some(path) = args.report {
        std::fs::write(path, &pretty)?;
    }
    if args.json || !args.dry_run {
        println!("{pretty}");
    }
    Ok(())
}

fn run_compression_office_office_inspect(
    args: CompressionOfficeOfficeArgs,
) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.input)?;
    let report =
        wellfriendpdf_engine::sdk::compression_office_office_inspect_json(&bytes, &args.format)?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_compression_office_office_to_pdf(
    args: CompressionOfficeOfficeToPdfArgs,
) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.input)?;
    let (output, report) =
        wellfriendpdf_engine::sdk::compression_office_office_to_pdf_json(&bytes, &args.format)?;
    if !args.dry_run {
        std::fs::write(&args.output, output)?;
    }
    let pretty = pretty_json(&report)?;
    if let Some(path) = args.report {
        std::fs::write(path, &pretty)?;
    }
    if args.json || !args.dry_run {
        println!("{pretty}");
    }
    Ok(())
}

fn run_advanced_editing_closeout_text_range(
    args: AdvancedEditingCloseoutTextRangeArgs,
) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let _selection_mode = if args.visual_selection {
        "visual_selection_resolved_to_logical"
    } else {
        "logical"
    };
    let _logical_requested = args.logical;
    if args.analyze {
        let report = wellfriendpdf_engine::sdk::advanced_editing_closeout_text_range_analyze_json(
            &bytes,
            args.page,
            args.password.as_deref().map(str::as_bytes),
        )?;
        return write_output_optional(&args.report, &pretty_json(&report)?);
    }
    let request = args
        .request
        .ok_or_else(|| usage_error("edit-text-range requires --request or --analyze"))?;
    let request_json = std::fs::read_to_string(request)?;
    let (output, report) =
        wellfriendpdf_engine::sdk::advanced_editing_closeout_text_range_edit_json(
            &bytes,
            &request_json,
            args.password.as_deref().map(str::as_bytes),
        )?;
    std::fs::write(args.output, output)?;
    let pretty = pretty_json(&report)?;
    if let Some(path) = args.report {
        std::fs::write(path, pretty)?;
    } else {
        println!("{pretty}");
    }
    Ok(())
}

fn run_source_editing_report(
    args: AnnotationMediaRedactionReportArgs,
) -> Result<(), Box<dyn Error>> {
    let bytes = read_edit_input(&args.pdf, &args.password)?;
    let report = wellfriendpdf_engine::sdk::source_editing_report_json(
        &bytes,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&None, &pretty_json(&report)?)
}

fn run_source_editing_provenance(
    args: SourceEditingTextSelectionArgs,
) -> Result<(), Box<dyn Error>> {
    let input = read_edit_input(&args.pdf, &args.password)?;
    let report = wellfriendpdf_engine::sdk::source_editing_provenance_json(
        &input,
        args.page,
        &args.source_text,
        &args.replacement_text,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_source_editing_eligibility(
    args: SourceEditingTextSelectionArgs,
) -> Result<(), Box<dyn Error>> {
    let input = read_edit_input(&args.pdf, &args.password)?;
    let request = serde_json::json!({
        "requested_mode": "operator_preserving",
        "page": args.page,
        "source_text": args.source_text,
        "replacement_text": args.replacement_text,
        "signature_policy_override": false
    });
    let report = wellfriendpdf_engine::sdk::source_editing_edit_eligibility_json(
        &input,
        &request.to_string(),
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_source_editing_text_edit(args: SourceEditingTextEditArgs) -> Result<(), Box<dyn Error>> {
    let input = read_edit_input(&args.pdf, &args.password)?;
    let request = serde_json::json!({
        "requested_mode": "operator_preserving",
        "page": args.page,
        "source_text": args.source_text,
        "replacement_text": args.replacement_text,
        "signature_policy_override": args.signature_policy_override
    });
    let (output, report) = wellfriendpdf_engine::sdk::source_editing_operator_text_edit_json(
        &input,
        &request.to_string(),
        args.password.as_deref().map(str::as_bytes),
    )?;
    std::fs::write(&args.output, output)?;
    let pretty = pretty_json(&report)?;
    if let Some(path) = args.report {
        std::fs::write(path, pretty)?;
    } else {
        println!("{pretty}");
    }
    Ok(())
}

fn run_source_editing_path_edit(args: SourceEditingPathEditArgs) -> Result<(), Box<dyn Error>> {
    let input = read_edit_input(&args.pdf, &args.password)?;
    let operation = if let Some(path) = args.operation {
        std::fs::read_to_string(path)?
    } else {
        "{}".to_string()
    };
    let options = serde_json::json!({
        "signature_policy_override": args.signature_policy_override,
        "deterministic": true,
        "shared_form_policy": args.shared_form_policy
    });
    let options_json = options.to_string();
    let (output, report) = wellfriendpdf_engine::sdk::source_editing_path_edit_json(
        &input,
        args.page,
        &args.id,
        &operation,
        Some(options_json.as_str()),
        args.password.as_deref().map(str::as_bytes),
    )?;
    std::fs::write(&args.output, output)?;
    let pretty = pretty_json(&report)?;
    if let Some(path) = args.report {
        std::fs::write(path, pretty)?;
    } else {
        println!("{pretty}");
    }
    Ok(())
}

fn run_source_editing_image_eligibility(
    args: SourceEditingImageArgs,
) -> Result<(), Box<dyn Error>> {
    let input = read_edit_input(&args.pdf, &args.password)?;
    let _ = args.occurrence.as_deref();
    let report = wellfriendpdf_engine::sdk::source_editing_image_eligibility_json(
        &input,
        args.page,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_editing_transactions_report(
    args: AnnotationMediaRedactionReportArgs,
) -> Result<(), Box<dyn Error>> {
    let bytes = read_edit_input(&args.pdf, &args.password)?;
    let report = wellfriendpdf_engine::sdk::editing_transactions_report_json(
        &bytes,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_editing_transactions_scene_report(
    args: EditingTransactionsSceneReportArgs,
) -> Result<(), Box<dyn Error>> {
    let input = read_edit_input(&args.pdf, &args.password)?;
    let pages = if args.page.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&args.page)?)
    };
    let report = wellfriendpdf_engine::sdk::editing_transactions_scene_report_json(
        &input,
        pages.as_deref(),
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_editing_transactions_scene_select(
    args: EditingTransactionsSceneSelectArgs,
) -> Result<(), Box<dyn Error>> {
    let input = read_edit_input(&args.pdf, &args.password)?;
    let point = match (args.x, args.y) {
        (Some(x), Some(y)) => Some([x, y]),
        (None, None) => None,
        _ => {
            return Err("--x and --y must be supplied together".into());
        }
    };
    let region = args
        .region
        .as_deref()
        .map(parse_editing_transactions_region)
        .transpose()?;
    let request = serde_json::json!({
        "page": args.page,
        "node_id": args.id,
        "point": point,
        "region": region,
        "cycle_index": args.cycle_index,
    });
    let report = wellfriendpdf_engine::sdk::editing_transactions_scene_select_json(
        &input,
        &request.to_string(),
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_editing_transactions_transaction_plan(
    args: EditingTransactionsTransactionArgs,
) -> Result<(), Box<dyn Error>> {
    let input = read_edit_input(&args.pdf, &args.password)?;
    let request = editing_transactions_request_json(&args)?;
    let report = wellfriendpdf_engine::sdk::editing_transactions_transaction_plan_json(
        &input,
        &request,
        args.password.as_deref().map(str::as_bytes),
    )?;
    if let Some(path) = args.report {
        std::fs::write(path, pretty_json(&report)?)?;
        Ok(())
    } else {
        write_output_optional(&None, &pretty_json(&report)?)
    }
}

fn run_editing_transactions_transaction_apply(
    args: EditingTransactionsTransactionArgs,
) -> Result<(), Box<dyn Error>> {
    let input = read_edit_input(&args.pdf, &args.password)?;
    let request = editing_transactions_request_json(&args)?;
    let (output, report) = wellfriendpdf_engine::sdk::editing_transactions_transaction_apply_json(
        &input,
        &request,
        args.password.as_deref().map(str::as_bytes),
    )?;
    std::fs::write(&args.output, output)?;
    let pretty = pretty_json(&report)?;
    if let Some(path) = args.report {
        std::fs::write(path, pretty)?;
    } else {
        println!("{pretty}");
    }
    Ok(())
}

fn run_editing_transactions_transaction_undo(
    args: EditingTransactionsTransactionArgs,
) -> Result<(), Box<dyn Error>> {
    let input = read_edit_input(&args.pdf, &args.password)?;
    let request = editing_transactions_request_json(&args)?;
    let plan_json = wellfriendpdf_engine::sdk::editing_transactions_transaction_plan_json(
        &input,
        &request,
        args.password.as_deref().map(str::as_bytes),
    )?;
    let report = serde_json::json!({
        "schema_version": wellfriendpdf_engine::REPORT_ENVELOPE_VERSION,
        "kind": "editing_transactions_transaction_undo",
        "report": {
            "schema_version": wellfriendpdf_engine::EDITING_TRANSACTIONS_SCHEMA_VERSION,
            "plan": serde_json::from_str::<serde_json::Value>(&plan_json)?["report"].clone(),
            "undo_policy": "exact_preimage_restore_or_declared_non_invertible_before_commit",
            "raw_preimage_not_logged": true,
            "redo_divergence_detection": "base_snapshot_id_and_source_instruction_hash_preconditions"
        }
    });
    write_output_optional(&args.report, &serde_json::to_string_pretty(&report)?)
}

fn run_editing_transactions_text_map(
    args: EditingTransactionsTextArgs,
) -> Result<(), Box<dyn Error>> {
    let report = wellfriendpdf_engine::sdk::editing_transactions_text_map_json(
        &args.text,
        args.direction.as_deref(),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_editing_transactions_shape_text(
    args: EditingTransactionsTextArgs,
) -> Result<(), Box<dyn Error>> {
    let report = wellfriendpdf_engine::sdk::editing_transactions_shape_text_json(
        &args.text,
        args.direction.as_deref(),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_editing_transactions_font_subset_plan(
    args: EditingTransactionsTextArgs,
) -> Result<(), Box<dyn Error>> {
    let report = wellfriendpdf_engine::sdk::editing_transactions_font_subset_plan_json(
        &args.text,
        args.direction.as_deref(),
        args.policy.as_deref(),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_editing_transactions_font_substitution_report(
    args: EditingTransactionsFontSubstitutionArgs,
) -> Result<(), Box<dyn Error>> {
    let report = wellfriendpdf_engine::sdk::editing_transactions_font_substitution_report_json(
        &args.requested_family,
        &args.text,
        args.policy.as_deref(),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_text_reflow_report(args: AnnotationMediaRedactionReportArgs) -> Result<(), Box<dyn Error>> {
    let bytes = read_edit_input(&args.pdf, &args.password)?;
    let report = wellfriendpdf_engine::sdk::text_reflow_report_json(
        &bytes,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_text_reflow_layout_analyze(args: TextReflowReflowArgs) -> Result<(), Box<dyn Error>> {
    let input = read_edit_input(&args.pdf, &args.password)?;
    let request = text_reflow_request_json(&args)?;
    let report = wellfriendpdf_engine::sdk::text_reflow_layout_analyze_json(
        &input,
        &request,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.json_output, &pretty_json(&report)?)
}

fn run_text_reflow_reading_order_report(
    args: AnnotationMediaRedactionReportArgs,
) -> Result<(), Box<dyn Error>> {
    let input = read_edit_input(&args.pdf, &args.password)?;
    let report = wellfriendpdf_engine::sdk::text_reflow_reading_order_report_json(
        &input,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_text_reflow_flow_graph_report(
    args: AnnotationMediaRedactionReportArgs,
) -> Result<(), Box<dyn Error>> {
    let input = read_edit_input(&args.pdf, &args.password)?;
    let report = wellfriendpdf_engine::sdk::text_reflow_flow_graph_report_json(
        &input,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_text_reflow_reflow_preview(args: TextReflowReflowArgs) -> Result<(), Box<dyn Error>> {
    let input = read_edit_input(&args.pdf, &args.password)?;
    let request = text_reflow_request_json(&args)?;
    let report = wellfriendpdf_engine::sdk::text_reflow_reflow_preview_json(
        &input,
        &request,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.json_output, &pretty_json(&report)?)
}

fn run_text_reflow_overflow_report(args: TextReflowReflowArgs) -> Result<(), Box<dyn Error>> {
    let input = read_edit_input(&args.pdf, &args.password)?;
    let request = text_reflow_request_json(&args)?;
    let report = wellfriendpdf_engine::sdk::text_reflow_overflow_report_json(
        &input,
        &request,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.json_output, &pretty_json(&report)?)
}

fn run_text_reflow_constraints_report(args: TextReflowReflowArgs) -> Result<(), Box<dyn Error>> {
    let input = read_edit_input(&args.pdf, &args.password)?;
    let request = text_reflow_request_json(&args)?;
    let report = wellfriendpdf_engine::sdk::text_reflow_constraints_report_json(
        &input,
        &request,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.json_output, &pretty_json(&report)?)
}

fn run_text_reflow_confidence_report(args: TextReflowReflowArgs) -> Result<(), Box<dyn Error>> {
    let input = read_edit_input(&args.pdf, &args.password)?;
    let request = text_reflow_request_json(&args)?;
    let report = wellfriendpdf_engine::sdk::text_reflow_confidence_report_json(
        &input,
        &request,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.json_output, &pretty_json(&report)?)
}

fn run_text_reflow_reflow_validate(args: TextReflowValidateArgs) -> Result<(), Box<dyn Error>> {
    let input = read_edit_input(&args.pdf, &args.password)?;
    let output = std::fs::read(&args.output_pdf)?;
    let request = std::fs::read_to_string(&args.request)?;
    let report = wellfriendpdf_engine::sdk::text_reflow_validate_reflow_output_json(
        &input,
        &output,
        &request,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_text_reflow_reflow_region(args: TextReflowReflowArgs) -> Result<(), Box<dyn Error>> {
    let input = read_edit_input(&args.pdf, &args.password)?;
    let request = text_reflow_request_json(&args)?;
    let (output, report) = wellfriendpdf_engine::sdk::text_reflow_reflow_region_json(
        &input,
        &request,
        args.password.as_deref().map(str::as_bytes),
    )?;
    std::fs::write(&args.output, output)?;
    let pretty = pretty_json(&report)?;
    if let Some(path) = args.report {
        std::fs::write(path, pretty)?;
    } else {
        println!("{pretty}");
    }
    Ok(())
}

fn run_text_reflow_reflow_document(args: TextReflowReflowArgs) -> Result<(), Box<dyn Error>> {
    let input = read_edit_input(&args.pdf, &args.password)?;
    let request = text_reflow_request_json(&args)?;
    let (output, report) = wellfriendpdf_engine::sdk::text_reflow_reflow_document_json(
        &input,
        &request,
        args.password.as_deref().map(str::as_bytes),
    )?;
    std::fs::write(&args.output, output)?;
    let pretty = pretty_json(&report)?;
    if let Some(path) = args.report {
        std::fs::write(path, pretty)?;
    } else {
        println!("{pretty}");
    }
    Ok(())
}

fn run_text_reflow_reflow_undo(args: TextReflowUndoArgs) -> Result<(), Box<dyn Error>> {
    if args.restored_pdf == args.pdf || args.restored_pdf == args.output_pdf {
        return Err(
            "text_reflow reflow-undo requires --restored-pdf distinct from both input PDFs".into(),
        );
    }
    if args.restored_pdf.exists() {
        return Err(
            "text_reflow reflow-undo refuses to overwrite an existing --restored-pdf".into(),
        );
    }
    if args.report.as_ref().is_some_and(|path| path.exists()) {
        return Err("text_reflow reflow-undo refuses to overwrite an existing --report".into());
    }
    let input = read_edit_input(&args.pdf, &args.password)?;
    let output = std::fs::read(&args.output_pdf)?;
    let request = std::fs::read_to_string(&args.request)?;
    let (restored, report) = wellfriendpdf_engine::sdk::text_reflow_undo_reflow_json(
        &input,
        &output,
        &request,
        args.password.as_deref().map(str::as_bytes),
    )?;
    std::fs::write(&args.restored_pdf, restored)?;
    let pretty = pretty_json(&report)?;
    if let Some(path) = args.report {
        std::fs::write(path, pretty)?;
    } else {
        println!("{pretty}");
    }
    Ok(())
}

fn run_text_reflow_reflow_approve_structure(
    args: TextReflowStructureCorrectionArgs,
) -> Result<(), Box<dyn Error>> {
    let input = read_edit_input(&args.pdf, &args.password)?;
    let correction = std::fs::read_to_string(args.correction)?;
    let report = wellfriendpdf_engine::sdk::text_reflow_reflow_approve_structure_json(
        &input,
        &correction,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_text_reflow_reflow_operation_report(
    args: TextReflowReflowArgs,
) -> Result<(), Box<dyn Error>> {
    let input = read_edit_input(&args.pdf, &args.password)?;
    let request = text_reflow_request_json(&args)?;
    let report = wellfriendpdf_engine::sdk::text_reflow_reflow_operation_report_json(
        &input,
        &request,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.json_output, &pretty_json(&report)?)
}

fn run_document_subsystems_report(
    args: AnnotationMediaRedactionReportArgs,
) -> Result<(), Box<dyn Error>> {
    let input = read_edit_input(&args.pdf, &args.password)?;
    let report = wellfriendpdf_engine::sdk::document_subsystems_report_json(
        &input,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_document_subsystems_analyze(
    args: AnnotationMediaRedactionReportArgs,
) -> Result<(), Box<dyn Error>> {
    let input = read_edit_input(&args.pdf, &args.password)?;
    let report = wellfriendpdf_engine::sdk::document_subsystems_analyze_json(
        &input,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_document_subsystems_plan(args: DocumentSubsystemsRequestArgs) -> Result<(), Box<dyn Error>> {
    let input = read_edit_input(&args.pdf, &args.password)?;
    let request = std::fs::read_to_string(args.request)?;
    let report = wellfriendpdf_engine::sdk::document_subsystems_plan_json(
        &input,
        &request,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_document_subsystems_apply(args: DocumentSubsystemsApplyArgs) -> Result<(), Box<dyn Error>> {
    if args.output == args.pdf {
        return Err(
            "document_subsystems-apply requires --output distinct from the input PDF".into(),
        );
    }
    if args.output.exists() {
        return Err("document_subsystems-apply refuses to overwrite an existing --output".into());
    }
    if args.report.as_ref().is_some_and(|path| path.exists()) {
        return Err("document_subsystems-apply refuses to overwrite an existing --report".into());
    }
    let input = read_edit_input(&args.pdf, &args.password)?;
    let request = std::fs::read_to_string(args.request)?;
    let (output, report) = wellfriendpdf_engine::sdk::document_subsystems_apply_json(
        &input,
        &request,
        args.password.as_deref().map(str::as_bytes),
    )?;
    std::fs::write(&args.output, output)?;
    if let Some(report_path) = args.report {
        std::fs::write(report_path, pretty_json(&report)?)?;
    } else {
        println!("{}", pretty_json(&report)?);
    }
    Ok(())
}

fn run_document_subsystems_undo(args: DocumentSubsystemsUndoArgs) -> Result<(), Box<dyn Error>> {
    if args.restored_pdf == args.pdf || args.restored_pdf == args.output_pdf {
        return Err(
            "document_subsystems-undo requires --restored-pdf distinct from both input PDFs".into(),
        );
    }
    if args.restored_pdf.exists() {
        return Err(
            "document_subsystems-undo refuses to overwrite an existing --restored-pdf".into(),
        );
    }
    if args.report.as_ref().is_some_and(|path| path.exists()) {
        return Err("document_subsystems-undo refuses to overwrite an existing --report".into());
    }
    let input = read_edit_input(&args.pdf, &args.password)?;
    let output = std::fs::read(args.output_pdf)?;
    let request = std::fs::read_to_string(args.request)?;
    let (restored, report) = wellfriendpdf_engine::sdk::document_subsystems_undo_json(
        &input,
        &output,
        &request,
        args.password.as_deref().map(str::as_bytes),
    )?;
    std::fs::write(&args.restored_pdf, restored)?;
    if let Some(report_path) = args.report {
        std::fs::write(report_path, pretty_json(&report)?)?;
    } else {
        println!("{}", pretty_json(&report)?);
    }
    Ok(())
}

fn run_document_security_report(
    args: AnnotationMediaRedactionReportArgs,
) -> Result<(), Box<dyn Error>> {
    let input = read_edit_input(&args.pdf, &args.password)?;
    let report = wellfriendpdf_engine::sdk::document_security_report_json(
        &input,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_document_security_analyze(
    args: AnnotationMediaRedactionReportArgs,
) -> Result<(), Box<dyn Error>> {
    let input = read_edit_input(&args.pdf, &args.password)?;
    let report = wellfriendpdf_engine::sdk::document_security_analyze_json(
        &input,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_document_security_plan(args: DocumentSecurityRequestArgs) -> Result<(), Box<dyn Error>> {
    let input = read_edit_input(&args.pdf, &args.password)?;
    let request = std::fs::read_to_string(args.request)?;
    let report = wellfriendpdf_engine::sdk::document_security_plan_json(
        &input,
        &request,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_document_security_apply(args: DocumentSecurityApplyArgs) -> Result<(), Box<dyn Error>> {
    if args.output == args.pdf {
        return Err("document_security-apply requires --output distinct from the input PDF".into());
    }
    if args.output.exists() {
        return Err("document_security-apply refuses to overwrite an existing --output".into());
    }
    if args.report.as_ref().is_some_and(|path| path.exists()) {
        return Err("document_security-apply refuses to overwrite an existing --report".into());
    }
    let input = read_edit_input(&args.pdf, &args.password)?;
    let request = std::fs::read_to_string(args.request)?;
    let (output, report) = wellfriendpdf_engine::sdk::document_security_apply_json(
        &input,
        &request,
        args.password.as_deref().map(str::as_bytes),
    )?;
    std::fs::write(&args.output, output)?;
    if let Some(report_path) = args.report {
        std::fs::write(report_path, pretty_json(&report)?)?;
    } else {
        println!("{}", pretty_json(&report)?);
    }
    Ok(())
}

fn run_document_security_undo(args: DocumentSecurityUndoArgs) -> Result<(), Box<dyn Error>> {
    if args.restored_pdf == args.pdf || args.restored_pdf == args.output_pdf {
        return Err(
            "document_security-undo requires --restored-pdf distinct from both input PDFs".into(),
        );
    }
    if args.restored_pdf.exists() {
        return Err(
            "document_security-undo refuses to overwrite an existing --restored-pdf".into(),
        );
    }
    if args.report.as_ref().is_some_and(|path| path.exists()) {
        return Err("document_security-undo refuses to overwrite an existing --report".into());
    }
    let input = read_edit_input(&args.pdf, &args.password)?;
    let output = std::fs::read(args.output_pdf)?;
    let request = std::fs::read_to_string(args.request)?;
    let (restored, report) = wellfriendpdf_engine::sdk::document_security_undo_json(
        &input,
        &output,
        &request,
        args.password.as_deref().map(str::as_bytes),
    )?;
    std::fs::write(&args.restored_pdf, restored)?;
    if let Some(report_path) = args.report {
        std::fs::write(report_path, pretty_json(&report)?)?;
    } else {
        println!("{}", pretty_json(&report)?);
    }
    Ok(())
}

fn run_document_security_verify_residual(
    args: DocumentSecurityVerifyArgs,
) -> Result<(), Box<dyn Error>> {
    let input = read_edit_input(&args.pdf, &args.password)?;
    let terms = std::fs::read_to_string(args.terms)?;
    let report = wellfriendpdf_engine::sdk::document_security_verify_residual_json(
        &input,
        &terms,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn text_reflow_request_json(args: &TextReflowReflowArgs) -> Result<String, Box<dyn Error>> {
    if let Some(path) = &args.request {
        return Ok(std::fs::read_to_string(path)?);
    }
    let region = args
        .region
        .as_deref()
        .map(parse_editing_transactions_region)
        .transpose()?;
    Ok(serde_json::json!({
        "requested_mode": args.mode,
        "page": args.page,
        "source_text": args.source_text,
        "replacement_text": args.replacement_text,
        "region": region,
        "language": args.language,
        "direction": args.direction,
        "font_policy": args.font_policy,
        "hyphenation": args.hyphenation,
        "allow_page_creation": args.allow_page_creation,
        "allow_font_reduction": false,
        "approve_low_confidence_structure": args.approve_low_confidence_structure,
        "signature_policy_override": args.signature_policy_override,
    })
    .to_string())
}

fn editing_transactions_request_json(
    args: &EditingTransactionsTransactionArgs,
) -> Result<String, Box<dyn Error>> {
    if let Some(path) = &args.request {
        return Ok(std::fs::read_to_string(path)?);
    }
    Ok(serde_json::json!({
        "requested_mode": "operator_preserving",
        "page": args.page,
        "source_text": args.source_text,
        "replacement_text": args.replacement_text,
        "signature_policy_override": args.signature_policy_override,
        "font_policy": args.font_policy,
        "normalization_policy": "preserve_exact_sequence",
        "direction": args.direction,
    })
    .to_string())
}

fn parse_editing_transactions_region(value: &str) -> Result<[f64; 4], Box<dyn Error>> {
    let values = value
        .split(',')
        .map(|item| item.trim().parse::<f64>())
        .collect::<Result<Vec<_>, _>>()?;
    if values.len() != 4 {
        return Err("--region must contain four comma-separated numbers".into());
    }
    Ok([values[0], values[1], values[2], values[3]])
}

fn run_advanced_editing_vector_list(
    args: AdvancedEditingVectorListArgs,
) -> Result<(), Box<dyn Error>> {
    let input = read_edit_input(&args.pdf, &args.password)?;
    let report = wellfriendpdf_engine::list_vector_objects(&input, args.page)?;
    write_output_optional(&args.output, &serde_json::to_string_pretty(&report)?)
}

fn run_advanced_editing_vector_edit(
    args: AdvancedEditingVectorEditArgs,
) -> Result<(), Box<dyn Error>> {
    let input = read_edit_input(&args.pdf, &args.password)?;
    let operation: wellfriendpdf_engine::VectorEditOperation =
        serde_json::from_str(&std::fs::read_to_string(&args.operation)?)?;
    let (output, report) = wellfriendpdf_engine::edit_vector_object(
        &input,
        args.page,
        &args.id,
        operation,
        &wellfriendpdf_engine::VectorEditOptions {
            signature_policy_override: args.signature_policy_override,
            deterministic: true,
            shared_form_policy: parse_advanced_editing_shared_form_policy(
                &args.shared_form_policy,
            )?,
        },
    )?;
    std::fs::write(&args.output, output)?;
    let json = serde_json::to_string_pretty(&report)?;
    if let Some(path) = args.report {
        std::fs::write(path, json)?;
    } else {
        println!("{json}");
    }
    Ok(())
}

fn run_advanced_editing_vector_direct(
    args: AdvancedEditingVectorDirectArgs,
    duplicate: bool,
) -> Result<(), Box<dyn Error>> {
    let input = read_edit_input(&args.pdf, &args.password)?;
    let operation = if duplicate {
        wellfriendpdf_engine::VectorEditOperation::Duplicate {
            dx: args.dx,
            dy: args.dy,
        }
    } else {
        wellfriendpdf_engine::VectorEditOperation::Delete
    };
    let (output, report) = wellfriendpdf_engine::edit_vector_object(
        &input,
        args.page,
        &args.id,
        operation,
        &wellfriendpdf_engine::VectorEditOptions {
            signature_policy_override: args.signature_policy_override,
            deterministic: true,
            shared_form_policy: parse_advanced_editing_shared_form_policy(
                &args.shared_form_policy,
            )?,
        },
    )?;
    std::fs::write(&args.output, output)?;
    let json = serde_json::to_string_pretty(&report)?;
    if let Some(path) = args.report {
        std::fs::write(path, json)?;
    } else {
        println!("{json}");
    }
    Ok(())
}

fn parse_advanced_editing_shared_form_policy(
    value: &str,
) -> Result<wellfriendpdf_engine::SharedFormEditPolicy, Box<dyn Error>> {
    match value {
        "reject" => Ok(wellfriendpdf_engine::SharedFormEditPolicy::Reject),
        "edit-all-uses" | "edit_all_uses" => Ok(wellfriendpdf_engine::SharedFormEditPolicy::EditAllUses),
        "clone-edit-one-instance" | "clone_edit_one_instance" => {
            Ok(wellfriendpdf_engine::SharedFormEditPolicy::CloneEditOneInstance)
        }
        other => Err(format!(
            "unsupported shared Form policy '{other}'; expected reject, edit-all-uses, or clone-edit-one-instance"
        )
        .into()),
    }
}

fn run_advanced_editing_ink_fit(args: AdvancedEditingInkFitArgs) -> Result<(), Box<dyn Error>> {
    let input = read_edit_input(&args.pdf, &args.password)?;
    let options = args
        .options
        .as_ref()
        .map(std::fs::read_to_string)
        .transpose()?
        .map(|json| serde_json::from_str::<wellfriendpdf_engine::InkFitOptions>(&json))
        .transpose()?
        .unwrap_or_default();
    let (output, report) = wellfriendpdf_engine::fit_annotation_ink_pdf(
        &input,
        args.page,
        args.annotation,
        &options,
        args.signature_policy_override,
    )?;
    std::fs::write(&args.output, output)?;
    let json = serde_json::to_string_pretty(&report)?;
    if let Some(path) = args.report {
        std::fs::write(path, json)?;
    } else {
        println!("{json}");
    }
    Ok(())
}

fn run_secure_mutation_redaction(
    args: NonAxisRedactionArgs,
    masked: bool,
) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let mut options_value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&args.plan)?)?;
    if args.promote {
        options_value["promote_inline_images"] = serde_json::Value::Bool(true);
    }
    let options = serde_json::to_string(&options_value)?;
    if args.dry_run {
        let report = wellfriendpdf_engine::sdk::nonaxis_redaction_plan_json(
            &bytes,
            &options,
            args.password.as_deref().map(str::as_bytes),
        )?;
        return write_xfa_operation_report(&report, args.report.as_ref(), true);
    }
    let (output, report) = if masked {
        wellfriendpdf_engine::sdk::redact_image_mask_json(
            &bytes,
            &options,
            args.password.as_deref().map(str::as_bytes),
        )?
    } else {
        wellfriendpdf_engine::sdk::redact_inline_image_json(
            &bytes,
            &options,
            args.password.as_deref().map(str::as_bytes),
        )?
    };
    std::fs::write(&args.output, output)?;
    write_xfa_operation_report(&report, args.report.as_ref(), args.json)
}

fn run_associated_files_report(
    args: AnnotationMediaRedactionReportArgs,
) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let report = wellfriendpdf_engine::sdk::associated_files_report_json(
        &bytes,
        args.password.as_deref().map(str::as_bytes),
    )?;
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_associated_files_extract(args: AssociatedFilesExtractArgs) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let (payload, report) = wellfriendpdf_engine::sdk::associated_files_extract_json(
        &bytes,
        &args.id,
        args.password.as_deref().map(str::as_bytes),
    )?;
    std::fs::write(args.output, payload)?;
    write_xfa_operation_report(&report, args.report.as_ref(), false)
}

fn run_associated_files_add(args: AssociatedFilesAddArgs) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let payload = std::fs::read(&args.file)?;
    let options = std::fs::read_to_string(&args.options)?;
    let (output, report) = wellfriendpdf_engine::sdk::associated_files_add_json(
        &bytes,
        &payload,
        &options,
        args.password.as_deref().map(str::as_bytes),
    )?;
    if !args.dry_run {
        std::fs::write(args.output, output)?;
    }
    write_xfa_operation_report(&report, args.report.as_ref(), args.json)
}

fn run_associated_files_update(args: AssociatedFilesUpdateArgs) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let payload = std::fs::read(&args.file)?;
    let options = std::fs::read_to_string(&args.options)?;
    let (output, report) = wellfriendpdf_engine::sdk::associated_files_update_owner_json(
        &bytes,
        &payload,
        &options,
        args.password.as_deref().map(str::as_bytes),
    )?;
    if !args.dry_run {
        std::fs::write(args.output, output)?;
    }
    write_xfa_operation_report(&report, args.report.as_ref(), args.json)
}

fn run_associated_files_remove(args: AssociatedFilesRemoveArgs) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let (output, report) = if let Some(owner) = &args.owner {
        if args.id.len() != 1 {
            return Err(usage_error("--owner requires exactly one --id"));
        }
        let options = serde_json::json!({
            "stable_id": args.id[0],
            "owner": owner.trim().to_ascii_lowercase().replace('-', "_"),
            "owner_ref": args.owner_ref,
        });
        wellfriendpdf_engine::sdk::associated_files_remove_owner_json(
            &bytes,
            &options.to_string(),
            args.password.as_deref().map(str::as_bytes),
        )?
    } else {
        wellfriendpdf_engine::sdk::associated_files_remove_json(
            &bytes,
            &args.id,
            args.password.as_deref().map(str::as_bytes),
        )?
    };
    if !args.dry_run {
        std::fs::write(args.output, output)?;
    }
    write_xfa_operation_report(&report, args.report.as_ref(), args.json)
}

fn signature_policy_override(value: &str) -> Result<bool, Box<dyn Error>> {
    match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "enforce" | "warn" | "allow_warning" => Ok(false),
        "override" | "explicit_override" => Ok(true),
        other => Err(usage_error(format!(
            "unknown signature policy '{other}'; use enforce, warn, or override"
        ))),
    }
}

fn run_edit_form(args: EditFormArgs) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let (output, report) = wellfriendpdf_engine::sdk::incremental_form_edit_json(
        &bytes,
        &args.field,
        &args.value,
        signature_policy_override(&args.signature_policy)?,
        args.password.as_deref().map(str::as_bytes),
    )?;
    if !args.dry_run {
        std::fs::write(args.output, output)?;
    }
    write_xfa_operation_report(&report, args.report.as_ref(), args.json)
}

fn run_signature_preserving_form(
    args: SignaturePreservingFormArgs,
    plan_only: bool,
) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let options_json = args
        .signature_options
        .as_ref()
        .map(std::fs::read_to_string)
        .transpose()?
        .unwrap_or_else(|| "{}".to_string());
    if plan_only {
        let report = wellfriendpdf_engine::sdk::signature_preserving_form_plan_json(
            &bytes,
            &args.field,
            &args.value,
            &options_json,
            args.password.as_deref().map(str::as_bytes),
        )?;
        if !args.json && args.report.is_none() {
            println!("{}", pretty_json(&report)?);
            return Ok(());
        }
        return write_xfa_operation_report(&report, args.report.as_ref(), args.json);
    }

    let (output, report) = wellfriendpdf_engine::sdk::signature_preserving_form_edit_json(
        &bytes,
        &args.field,
        &args.value,
        &options_json,
        signature_policy_override(&args.signature_policy)?,
        args.password.as_deref().map(str::as_bytes),
    )?;
    if !args.dry_run {
        std::fs::write(&args.output, output)?;
    }
    if !args.json && args.report.is_none() {
        println!(
            "Wrote signature-preserving form edit to {}",
            args.output.display()
        );
    }
    write_xfa_operation_report(&report, args.report.as_ref(), args.json)
}

fn run_edit_mutation(args: EditMutationArgs, annotation: bool) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let options = std::fs::read_to_string(&args.options)?;
    let override_policy = signature_policy_override(&args.signature_policy)?;
    let (output, report) = if annotation {
        wellfriendpdf_engine::sdk::incremental_annotation_edit_json(
            &bytes,
            &options,
            override_policy,
            args.password.as_deref().map(str::as_bytes),
        )?
    } else {
        wellfriendpdf_engine::sdk::incremental_page_property_edit_json(
            &bytes,
            &options,
            override_policy,
            args.password.as_deref().map(str::as_bytes),
        )?
    };
    if !args.dry_run {
        std::fs::write(args.output, output)?;
    }
    write_xfa_operation_report(&report, args.report.as_ref(), args.json)
}

fn run_associated_files_sanitize(args: AssociatedFilesSanitizeArgs) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let options = args
        .options
        .as_ref()
        .map(std::fs::read_to_string)
        .transpose()?;
    let (output, report) = wellfriendpdf_engine::sdk::associated_files_sanitize_json(
        &bytes,
        options.as_deref(),
        args.password.as_deref().map(str::as_bytes),
    )?;
    if !args.dry_run {
        std::fs::write(args.output, output)?;
    }
    write_xfa_operation_report(&report, args.report.as_ref(), args.json)
}

fn run_edit_policy_report(args: EditPolicyArgs, impact: bool) -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(&args.pdf)?;
    let report = if impact {
        wellfriendpdf_engine::sdk::edit_signature_impact_json(
            &bytes,
            &args.operation,
            args.password.as_deref().map(str::as_bytes),
        )?
    } else {
        wellfriendpdf_engine::sdk::edit_policy_report_json(
            &bytes,
            &args.operation,
            args.password.as_deref().map(str::as_bytes),
        )?
    };
    write_output_optional(&args.output, &pretty_json(&report)?)
}

fn run_forms_export(args: FormsExportArgs) -> Result<(), Box<dyn Error>> {
    let engine = open_engine(&args.pdf, &args.password)?;
    let format = parse_form_data_format(&args.format)?;
    let bytes = wellfriendpdf_engine::export_form_data(&engine, format)?;
    match args.output {
        Some(path) => std::fs::write(path, bytes)?,
        None => {
            if matches!(
                format,
                wellfriendpdf_engine::FormDataFormat::Json
                    | wellfriendpdf_engine::FormDataFormat::Xfdf
            ) {
                print!("{}", String::from_utf8_lossy(&bytes));
            } else {
                return Err(usage_error(
                    "forms-export --format fdf requires --output because FDF is binary-safe PDF syntax",
                ));
            }
        }
    }
    Ok(())
}

fn run_forms_import(args: FormsImportArgs) -> Result<(), Box<dyn Error>> {
    let format = if let Some(format) = args.format.as_deref() {
        parse_form_data_format(format)?
    } else {
        let extension = args
            .data
            .extension()
            .and_then(|s| s.to_str())
            .ok_or_else(|| usage_error("forms-import needs --format when data has no extension"))?;
        parse_form_data_format(extension)?
    };
    let input = read_edit_input(&args.pdf, &args.password)?;
    let data = std::fs::read(&args.data)?;
    let (bytes, report) = wellfriendpdf_engine::apply_form_data_pdf(input, &data, format)?;
    std::fs::write(&args.output, &bytes)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        eprintln!(
            "Imported {} field(s), applied {} -> {}",
            report.imported_fields,
            report.applied_fields,
            args.output.display()
        );
    }
    Ok(())
}

fn run_annotations_report(args: AnnotationsReportArgs) -> Result<(), Box<dyn Error>> {
    let engine = open_engine(&args.pdf, &args.password)?;
    let output = serde_json::to_string_pretty(&wellfriendpdf_engine::annotation_report(&engine)?)?;
    write_output_optional(&args.output, &output)?;
    Ok(())
}

fn run_annotations_flatten(args: AnnotationsFlattenArgs) -> Result<(), Box<dyn Error>> {
    let input = read_edit_input(&args.pdf, &args.password)?;
    let mut editor = wellfriendpdf_engine::PdfEditor::open_bytes(input)?;
    editor.flatten_annotations();
    let bytes = editor.save_to_bytes(wellfriendpdf_engine::EditMode::FullRewrite)?;
    std::fs::write(&args.output, &bytes)?;
    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "op": "annotations-flatten",
                "output": args.output.display().to_string(),
                "bytes": bytes.len(),
                "diagnostics": ["annotations.flatten.common_appearance_subset"]
            })
        );
    } else {
        eprintln!("Flattened annotations -> {}", args.output.display());
    }
    Ok(())
}

fn run_pages_report(args: PagesReportArgs) -> Result<(), Box<dyn Error>> {
    let engine = open_engine(&args.pdf, &args.password)?;
    let output =
        serde_json::to_string_pretty(&wellfriendpdf_engine::page_operations_report(&engine)?)?;
    write_output_optional(&args.output, &output)?;
    Ok(())
}

fn run_interactive_report(args: InteractiveReportArgs) -> Result<(), Box<dyn Error>> {
    let engine = open_engine(&args.pdf, &args.password)?;
    let output = serde_json::to_string_pretty(&wellfriendpdf_engine::interactive_report(&engine)?)?;
    write_output_optional(&args.output, &output)?;
    Ok(())
}

fn run_redact(args: RedactArgs) -> Result<(), Box<dyn Error>> {
    if args.text.is_empty() && args.rects.is_empty() {
        return Err("redact requires at least one --text or --rect".into());
    }
    let input = read_edit_input(&args.pdf, &args.password)?;
    let engine = wellfriendpdf_engine::ContentEngine::open_bytes(input.clone())?;
    let total = engine.page_count()?;
    let search_pages = parse_page_range_cli(&args.pages, total)?;
    let mut explicit_regions = Vec::new();
    for spec in &args.rects {
        explicit_regions.push(parse_redact_rect_cli(spec, total)?);
    }

    let mut editor = wellfriendpdf_engine::PdfEditor::open_bytes(input)?;
    let image_policy = parse_image_redaction_policy(&args.image_policy)?;
    let attachment_policy = parse_attachment_policy(&args.attachments)?;
    let redaction_options = wellfriendpdf_engine::RedactionOptions {
        fill: wellfriendpdf_engine::Color::black(),
        scrub_metadata: !args.no_metadata_scrub,
        image_policy,
        attachment_policy,
        promote_inline_images: false,
    };
    let mut search_regions = Vec::new();
    for term in &args.text {
        if term.trim().is_empty() {
            continue;
        }
        let matches = engine.search_text(
            &search_pages,
            term,
            wellfriendpdf_engine::TextSearchOptions {
                case_sensitive: false,
                include_hidden: true,
                ..wellfriendpdf_engine::TextSearchOptions::default()
            },
        )?;
        for hit in matches {
            if let Some(rect) = redaction_rect_from_quads(&hit.quads) {
                editor.redact(hit.page, rect, redaction_options.clone())?;
                search_regions.push(serde_json::json!({
                    "term": term,
                    "page": hit.page,
                    "rect": [rect.x, rect.y, rect.width, rect.height],
                    "provenance": hit.provenance,
                    "role": hit.role,
                    "includes_hidden": hit.includes_hidden,
                }));
            }
        }
    }
    for region in &explicit_regions {
        editor.redact(region.page, region.rect, redaction_options.clone())?;
    }
    let redact_count = search_regions.len() + explicit_regions.len();
    if redact_count == 0 {
        return Err("redact found no matching text and no usable rectangles".into());
    }

    let bytes = editor.save_to_bytes(wellfriendpdf_engine::EditMode::FullRewrite)?;
    let verification = wellfriendpdf_engine::redaction_verification_report(&bytes, &args.text)?;
    if args.strict && !verification.verified_absent {
        return Err("strict redaction verification failed: requested term remains".into());
    }
    std::fs::write(&args.output, &bytes)?;

    let summary = serde_json::json!({
        "op": "redact",
        "output": args.output.display().to_string(),
        "bytes": bytes.len(),
        "search_terms": args.text,
        "search_regions": search_regions,
        "explicit_regions": explicit_regions.iter().map(|r| {
            serde_json::json!({
                "page": r.page,
                "rect": [r.rect.x, r.rect.y, r.rect.width, r.rect.height],
            })
        }).collect::<Vec<_>>(),
        "metadata_scrub": !args.no_metadata_scrub,
        "image_policy": args.image_policy,
        "attachment_policy": args.attachments,
        "verification": verification,
    });
    if args.json {
        println!("{}", serde_json::to_string_pretty(&summary)?);
    } else {
        eprintln!(
            "Redacted {} region(s) -> {}",
            redact_count,
            args.output.display()
        );
    }
    Ok(())
}

/// Split a PDF into RAG-ready semantic chunks (the embedding-pipeline surface).
fn run_chunk(args: ChunkArgs) -> Result<(), Box<dyn Error>> {
    use wellfriendpdf_engine::{ChunkOptions, ParseOptions};

    if !matches!(args.format.to_lowercase().as_str(), "json") {
        return Err(format!("unknown --format '{}'; only json is supported", args.format).into());
    }

    let engine = open_engine(&args.pdf, &args.password)?;
    let total = engine.page_count()?;
    let page_nums = parse_page_range_cli(&args.pages, total)?;

    if !args.dictionary_packs.is_empty() && !args.advanced {
        return Err("--dictionary-pack requires --advanced".into());
    }
    if args.advanced {
        let mode = wellfriendpdf_engine::AdvancedChunkMode::parse(&args.mode).ok_or_else(|| {
            format!(
                "unknown advanced chunk mode '{}'; use hybrid, page, section, paragraph, table, table-row, table-cell, figure-caption, cjk, or search-index",
                args.mode
            )
        })?;
        let report =
            engine.semantic_binding_report(&wellfriendpdf_engine::SemanticBindingOptions {
                pages: page_nums,
                dictionary_manifest_paths: args.dictionary_packs,
                chunk_options: wellfriendpdf_engine::AdvancedChunkOptions {
                    mode,
                    target_tokens: args.target_tokens.max(1),
                    overlap_tokens: args.overlap,
                    include_heading_context: !args.no_heading_context,
                    include_furniture: args.keep_furniture,
                    cjk_token_aware: mode == wellfriendpdf_engine::AdvancedChunkMode::CjkTokenAware,
                    ..wellfriendpdf_engine::AdvancedChunkOptions::default()
                },
                ..wellfriendpdf_engine::SemanticBindingOptions::default()
            })?;
        let output_text = serde_json::to_string_pretty(&report.rag_chunks)?;
        match &args.output {
            Some(path) => std::fs::write(path, &output_text)?,
            None => println!("{output_text}"),
        }
        eprintln!(
            "Advanced semantic chunking produced {} chunk(s) in {:?} mode.",
            report.rag_chunks.chunks.len(),
            mode
        );
        return Ok(());
    }

    // Parse once into the canonical model (OCR scanned pages when requested);
    // keep furniture in the model so chunking can decide per its own option.
    let mut parse_opts = ParseOptions {
        pages: page_nums,
        omit_furniture: false,
        ..ParseOptions::default()
    };
    if let Some(policy) = ocr_policy_from_flag(&args.ocr)? {
        parse_opts.ocr = Some(build_ocr_engine()?);
        parse_opts.ocr_policy = policy;
        parse_opts.ocr_options = ocr_options(&args.ocr_lang, args.ocr_dpi);
        parse_opts.ocr_dpi = args.ocr_dpi.max(1);
    }
    let document = engine.parse_document(&parse_opts)?;

    let chunk_opts = ChunkOptions {
        target_tokens: args.target_tokens.max(1),
        overlap_tokens: args.overlap,
        heading_context: !args.no_heading_context,
        split_on_headings: !args.no_split_on_headings,
        include_furniture: args.keep_furniture,
        isolate_tables: true,
    };
    let set = document.chunk(&chunk_opts);
    let output_text = set.to_json();

    match &args.output {
        Some(path) => std::fs::write(path, &output_text)?,
        None => {
            print!("{output_text}");
            if !output_text.ends_with('\n') {
                println!();
            }
        }
    }
    let total_tokens: usize = set.chunks.iter().map(|c| c.tokens).sum();
    let avg = if set.chunks.is_empty() {
        0
    } else {
        total_tokens / set.chunks.len()
    };
    let oversized = set.chunks.iter().filter(|c| c.oversized).count();
    eprintln!(
        "Chunked into {} chunk(s), ~{} tokens avg (target {}){}.",
        set.chunks.len(),
        avg,
        chunk_opts.target_tokens,
        if oversized > 0 {
            format!(", {oversized} oversized")
        } else {
            String::new()
        },
    );
    Ok(())
}

fn run_semantic_export(args: SemanticExportArgs) -> Result<(), Box<dyn Error>> {
    let engine = open_engine(&args.pdf, &args.password)?;
    let total = engine.page_count()?;
    let pages = parse_page_range_cli(&args.pages, total)?;
    let mode = wellfriendpdf_engine::AdvancedChunkMode::parse(&args.chunk_mode).ok_or_else(|| {
        format!(
            "unknown advanced chunk mode '{}'; use hybrid, page, section, paragraph, table, table-row, table-cell, figure-caption, cjk, or search-index",
            args.chunk_mode
        )
    })?;
    let table_proposals = match args.table_proposals {
        Some(path) => Some(serde_json::from_slice::<
            wellfriendpdf_engine::TableProposalSet,
        >(&std::fs::read(path)?)?),
        None => None,
    };
    let options = wellfriendpdf_engine::SemanticBindingOptions {
        pages,
        dictionary_manifest_paths: args.dictionary_packs,
        chunk_options: wellfriendpdf_engine::AdvancedChunkOptions {
            mode,
            target_tokens: args.target_tokens.max(1),
            overlap_tokens: args.overlap,
            cjk_token_aware: mode == wellfriendpdf_engine::AdvancedChunkMode::CjkTokenAware,
            ..wellfriendpdf_engine::AdvancedChunkOptions::default()
        },
        search_query: args.query.clone(),
        table_proposals,
        ..wellfriendpdf_engine::SemanticBindingOptions::default()
    };
    let report = engine.semantic_binding_report(&options)?;
    let value = match args.view.trim().to_ascii_lowercase().as_str() {
        "bundle" => serde_json::to_value(&report)?,
        "summary" => serde_json::json!({
            "schema_version": report.schema_version,
            "summary": report.summary,
            "privacy": report.privacy,
            "diagnostics": report.diagnostics,
        }),
        "semantic" | "semantic-json" => serde_json::json!({
            "schema_version": report.schema_version,
            "document": report.document,
            "text_semantic": report.text_semantic,
            "semantic_document": report.semantic_document,
            "parenttree_recovery": report.parenttree_recovery,
        }),
        "tables" | "table-json" => serde_json::json!({
            "schema_version": report.schema_version,
            "tables": report.tables,
            "table_model_backend_status": report.table_model_backend_status,
            "table_proposal_merge": report.table_proposal_merge,
        }),
        "tokens" | "cjk" => serde_json::json!({
            "schema_version": report.schema_version,
            "dictionary_report": report.dictionary_report,
            "pages": report.cjk_token_pages,
        }),
        "chunks" | "rag" => serde_json::to_value(&report.rag_chunks)?,
        "search" => {
            let query = args
                .query
                .as_deref()
                .map(str::trim)
                .filter(|query| !query.is_empty())
                .ok_or("--view search requires a non-empty --query")?;
            let query_folded = query.to_lowercase();
            let cjk_token_matches = report
                .cjk_token_pages
                .iter()
                .flat_map(|page| {
                    let query_folded = query_folded.clone();
                    page.tokens.iter().filter_map(move |token| {
                        let token_folded = token.text.to_lowercase();
                        (token_folded == query_folded || token_folded.contains(&query_folded)).then(
                            || {
                                serde_json::json!({
                                    "page": page.page,
                                    "text": token.text,
                                    "char_range": token.char_range,
                                    "byte_range": token.byte_range,
                                    "language": token.language,
                                    "confidence": token.confidence,
                                    "source": token.source,
                                })
                            },
                        )
                    })
                })
                .collect::<Vec<_>>();
            serde_json::json!({
                "schema_version": "semantic_closeout.semantic_search.v1",
                "query": query,
                "semantic_matches": report.search_results,
                "cjk_token_matches": cjk_token_matches,
                "dictionary_report": report.dictionary_report,
                "raw_text_fallback": true,
                "provenance_preserved": true,
            })
        }
        "status" | "ml-status" => serde_json::json!({
            "layout_backend_status": report.layout_backend_status,
            "table_model_backend_status": report.table_model_backend_status,
            "privacy": report.privacy,
        }),
        other => {
            return Err(format!(
                "unknown --view '{other}'; use bundle, summary, semantic, tables, tokens, chunks, search, or status"
            )
            .into());
        }
    };
    let output = serde_json::to_string_pretty(&value)?;
    match args.output {
        Some(path) => std::fs::write(path, &output)?,
        None => println!("{output}"),
    }
    Ok(())
}

/// Score an extraction result vs ground truth (the benchmark scoring core).
fn run_eval_score(args: EvalScoreArgs) -> Result<(), Box<dyn Error>> {
    use std::io::Read;
    let input_json = match &args.input {
        Some(path) => std::fs::read_to_string(path)?,
        None => {
            let mut s = String::new();
            std::io::stdin().read_to_string(&mut s)?;
            s
        }
    };
    let output_json = wellfriendpdf_engine::score_json(&input_json)
        .map_err(|e| -> Box<dyn Error> { e.into() })?;
    match &args.output {
        Some(path) => std::fs::write(path, &output_json)?,
        None => println!("{output_json}"),
    }
    Ok(())
}

fn run_document_model(args: DocumentModelArgs) -> Result<(), Box<dyn Error>> {
    let engine = open_engine(&args.pdf, &args.password)?;
    let total = engine.page_count()?;
    let page_nums = parse_page_range_cli(&args.pages, total)?;

    let as_json = match args.format.to_lowercase().as_str() {
        "json" => true,
        "md" | "markdown" | "text" | "txt" => false,
        other => {
            return Err(format!("unknown --format '{other}'; use json, md, or text").into());
        }
    };

    let mut model = engine.build_document_model(&page_nums)?;
    if args.min_confidence > 0.0 {
        model.blocks.retain(|b| b.confidence >= args.min_confidence);
        // Re-densify the reading-order indices after filtering; ids are kept so
        // any caption/figure cross-links remain resolvable.
        for (i, b) in model.blocks.iter_mut().enumerate() {
            b.reading_order_index = i;
        }
    }

    let output_text = if as_json {
        serde_json::to_string_pretty(&model)?
    } else {
        wellfriendpdf_engine::render_document_markdown(&model)
    };

    match &args.output {
        Some(path) => std::fs::write(path, &output_text)?,
        None => {
            print!("{output_text}");
            if !output_text.ends_with('\n') {
                println!();
            }
        }
    }
    eprintln!(
        "Document model: {} block(s) across {} page(s) ({:?} source)",
        model.blocks.len(),
        model.page_count,
        model.source
    );
    Ok(())
}

fn run_extract_images(args: ExtractImagesArgs) -> Result<(), Box<dyn Error>> {
    use std::io::Write;
    use wellfriendpdf_engine::{ImageLocateOptions, ImageLocator, ImageOutputFormat};
    use zip::{write::FileOptions, CompressionMethod, ZipWriter};

    let engine = open_engine(&args.pdf, &args.password)?;
    let total = engine.page_count()?;
    let page_nums = parse_page_range_cli(&args.pages, total)?;
    let selected_pages = page_nums.len();
    let region = args.region.as_deref().map(parse_region_cli).transpose()?;

    let format = match args.format.to_lowercase().as_str() {
        "png" => ImageOutputFormat::Png,
        "jpg" | "jpeg" => ImageOutputFormat::Jpeg,
        "webp" => ImageOutputFormat::Webp,
        "original" | "" => ImageOutputFormat::Original,
        other => {
            return Err(format!(
                "unknown format '{}'; use png, jpg, webp, or original",
                other
            )
            .into())
        }
    };

    let images: Vec<wellfriendpdf_engine::PlacedImageReference> = if let Some(region) = region {
        let mut placed = Vec::new();
        for page_num in page_nums {
            placed.extend(
                engine
                    .find_page_images_in_region(page_num, region)?
                    .into_iter()
                    .filter(|image| {
                        image.image.width >= args.min_width && image.image.height >= args.min_height
                    }),
            );
        }
        placed
    } else {
        let opts = ImageLocateOptions {
            pages: Some(page_nums),
            min_width: args.min_width,
            min_height: args.min_height,
            ..Default::default()
        };
        ImageLocator::find_all_images(&engine, &opts)?
            .into_iter()
            .map(|image| wellfriendpdf_engine::PlacedImageReference {
                image,
                bbox: [0.0; 4],
            })
            .collect()
    };

    let out_file = std::fs::File::create(&args.output)?;
    let mut zip = ZipWriter::new(out_file);
    let zip_opts = FileOptions::<()>::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(6));

    let mut encoded_count = 0usize;
    for (idx, placed) in images.iter().enumerate() {
        let img_ref = &placed.image;
        // Inline images (object_number == 0 with captured data) are exported too;
        // only skip references that carry no usable data.
        if img_ref.object_number == 0 && img_ref.inline_data.is_none() {
            continue;
        }

        let bytes = match engine.extract_image_bytes(img_ref, format.clone(), Some(args.quality)) {
            Ok(bytes) => bytes,
            Err(err) => {
                eprintln!(
                    "Warning: skipped image {} on page {}: {}",
                    img_ref.xobject_name, img_ref.page_number, err
                );
                continue;
            }
        };

        // For inline images, the chosen output extension is used as-is; XObject
        // images follow the same naming. The "-inline" marker keeps inline
        // exports recognizable without disturbing XObject numbering.
        let ext = if matches!(format, ImageOutputFormat::Original) && img_ref.is_inline {
            "png"
        } else {
            format.file_extension()
        };
        let suffix = if img_ref.is_inline { "-inline" } else { "" };
        let filename = format!(
            "page-{:03}-image-{:03}{}.{}",
            img_ref.page_number,
            idx + 1,
            suffix,
            ext
        );
        zip.start_file(&filename, zip_opts)?;
        zip.write_all(&bytes)?;
        encoded_count += 1;
    }
    zip.finish()?;

    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "op": "extract-images",
                "output": args.output.display().to_string(),
                "format": args.format,
                "pages": selected_pages,
                "images": encoded_count,
            })
        );
    } else {
        eprintln!(
            "Extracted {} image(s) -> {}",
            encoded_count,
            args.output.display()
        );
    }
    Ok(())
}

fn run_render_compare(args: RenderCompareArgs) -> Result<(), Box<dyn Error>> {
    use wellfriendpdf_engine::RenderMode;

    let dpi = args.dpi.clamp(24, 600);
    if dpi != args.dpi {
        eprintln!("Warning: DPI clamped to {} (valid range: 24-600)", dpi);
    }

    let render_mode = RenderMode::from_name(&args.render_quality)
        .ok_or_else(|| format!("unknown render quality '{}'", args.render_quality))?;
    let engine = open_engine(&args.pdf, &args.password)?;
    let total = engine.page_count()?;
    let pages = parse_page_range_cli(&args.pages, total)?;

    let mut totals_fallback_reasons = std::collections::BTreeMap::<String, usize>::new();
    let mut totals = serde_json::json!({
        "operations": 0usize,
        "text_ops": 0usize,
        "image_xobjects": 0usize,
        "inline_images": 0usize,
        "form_xobjects": 0usize,
        "native_text_ops": 0usize,
        "native_image_xobjects": 0usize,
        "native_inline_images": 0usize,
        "native_form_xobjects": 0usize,
        "compatibility_runs": 0usize,
        "compatibility_ops": 0usize,
        "unsupported_ops": 0usize,
    });
    let mut page_reports = Vec::new();

    for page in &pages {
        let list = engine.build_page_display_list(*page, dpi)?;
        let rendered = engine.render_page_display_list_with_mode(*page, dpi, render_mode)?;
        let stats = &list.stats;
        for (reason, count) in &stats.compatibility_fallback_reasons {
            *totals_fallback_reasons.entry(reason.clone()).or_insert(0) += count;
        }

        for (key, value) in [
            ("operations", stats.operations),
            ("text_ops", stats.text_ops),
            ("image_xobjects", stats.image_xobjects),
            ("inline_images", stats.inline_images),
            ("form_xobjects", stats.form_xobjects),
            ("native_text_ops", stats.native_text_ops),
            ("native_image_xobjects", stats.native_image_xobjects),
            ("native_inline_images", stats.native_inline_images),
            ("native_form_xobjects", stats.native_form_xobjects),
            ("compatibility_runs", stats.compatibility_runs),
            ("compatibility_ops", stats.compatibility_ops),
            ("unsupported_ops", stats.unsupported_ops),
        ] {
            if let Some(slot) = totals.get_mut(key) {
                *slot = serde_json::json!(slot.as_u64().unwrap_or(0) + value as u64);
            }
        }

        let render = match rendered {
            Some(buffer) => serde_json::json!({
                "status": "display_list_rendered",
                "width": buffer.width,
                "height": buffer.height,
            }),
            None => serde_json::json!({
                "status": "not_replayable",
            }),
        };
        let unsupported = list
            .unsupported
            .iter()
            .map(|op| {
                serde_json::json!({
                    "operator": &op.operator,
                    "reason": &op.reason,
                })
            })
            .collect::<Vec<_>>();

        page_reports.push(serde_json::json!({
            "page": page,
            "display_list": {
                "fully_supported": list.is_fully_supported(),
                "has_compatibility_runs": list.has_compatibility_runs(),
                "approximate_memory_bytes": list.approximate_memory_bytes(),
                "stats": {
                    "operations": stats.operations,
                    "saves": stats.saves,
                    "restores": stats.restores,
                    "clips": stats.clips,
                    "fills": stats.fills,
                    "strokes": stats.strokes,
                    "paths": stats.paths,
                    "path_segments": stats.path_segments,
                    "text_ops": stats.text_ops,
                    "image_xobjects": stats.image_xobjects,
                    "inline_images": stats.inline_images,
                    "form_xobjects": stats.form_xobjects,
                    "shadings": stats.shadings,
                    "patterns": stats.patterns,
                    "transparency_ops": stats.transparency_ops,
                    "native_text_ops": stats.native_text_ops,
                    "native_image_xobjects": stats.native_image_xobjects,
                    "native_inline_images": stats.native_inline_images,
                    "native_form_xobjects": stats.native_form_xobjects,
                    "compatibility_runs": stats.compatibility_runs,
                    "compatibility_ops": stats.compatibility_ops,
                    "compatibility_bytes": stats.compatibility_bytes,
                    "compatibility_fallback_reasons": stats.compatibility_fallback_reasons.clone(),
                    "unsupported_ops": stats.unsupported_ops,
                    "max_stack_depth": stats.max_stack_depth,
                },
                "unsupported": unsupported,
            },
            "render": render,
        }));
    }

    let report = serde_json::json!({
        "schema_version": 1,
        "kind": "render_compare",
        "feature_area": "combined_native_renderer",
        "input": args.pdf.display().to_string(),
        "dpi": dpi,
        "render_quality": args.render_quality,
        "pages": pages,
        "totals": totals,
        "compatibility_fallback_reasons": totals_fallback_reasons,
        "page_reports": page_reports,
    });
    let output = if args.pretty {
        serde_json::to_string_pretty(&report)?
    } else {
        serde_json::to_string(&report)?
    };
    write_output_optional(&args.output, &output)
}

fn run_render(args: RenderArgs) -> Result<(), Box<dyn Error>> {
    use rayon::prelude::*;
    use std::io::Write;
    use wellfriendpdf_engine::{ImageEncoder, ImageOutputFormat, RenderMode};
    use zip::{write::FileOptions, CompressionMethod, ZipWriter};

    let dpi = args.dpi.clamp(24, 600);
    if dpi != args.dpi {
        eprintln!("Warning: DPI clamped to {} (valid range: 24-600)", dpi);
    }

    // Honor an explicit per-page pixel cap by exporting it for the engine's
    // `max_render_pixels()` resolver (also read by the svg/ps/eps sub-paths,
    // which all size their pages through `page_viewport`).
    if let Some(cap) = args.max_render_pixels {
        std::env::set_var("WELLFRIENDPDF_MAX_RENDER_PIXELS", cap.to_string());
    }

    // Vector output formats take separate paths.
    match args.format.to_lowercase().as_str() {
        "svg" => return run_render_svg(args, dpi),
        "ps" => return run_render_ps(args, dpi),
        "eps" => return run_render_eps(args, dpi),
        _ => {}
    }

    let format = match args.format.to_lowercase().as_str() {
        "png" => ImageOutputFormat::Png,
        "jpg" | "jpeg" => ImageOutputFormat::Jpeg,
        "webp" => ImageOutputFormat::Webp,
        other => {
            return Err(format!(
                "unknown format '{}'; use png, jpg, webp, svg, ps, or eps",
                other
            )
            .into())
        }
    };

    let engine = open_engine(&args.pdf, &args.password)?;
    let total = engine.page_count()?;
    let page_nums = parse_page_range_cli(&args.pages, total)?;
    let render_mode = RenderMode::from_name(&args.render_quality)
        .ok_or_else(|| format!("unknown render quality '{}'", args.render_quality))?;

    let out_file = std::fs::File::create(&args.output)?;
    let mut zip = ZipWriter::new(out_file);
    let zip_opts = FileOptions::<()>::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(6));

    const PARALLEL_RENDER_PAGE_THRESHOLD: usize = 32;
    let quality = args.quality;
    let encode_page = |page_num: usize| -> Result<Vec<u8>, String> {
        let buf = engine
            .render_page_with_mode(page_num, dpi, render_mode)
            .map_err(|err| err.to_string())?;
        let raw = buf.to_raw_image();
        match &format {
            ImageOutputFormat::Jpeg => ImageEncoder::encode_jpeg(&raw, quality),
            ImageOutputFormat::Webp => ImageEncoder::encode_webp(&raw, quality),
            ImageOutputFormat::Png | ImageOutputFormat::Original => {
                ImageEncoder::encode_png_fast(&raw)
            }
        }
        .map_err(|err| err.to_string())
    };

    let rendered_pages: Vec<(usize, Result<Vec<u8>, String>)> =
        if page_nums.len() >= PARALLEL_RENDER_PAGE_THRESHOLD {
            page_nums
                .par_iter()
                .map(|&page_num| (page_num, encode_page(page_num)))
                .collect()
        } else {
            page_nums
                .iter()
                .map(|&page_num| (page_num, encode_page(page_num)))
                .collect()
        };

    let mut rendered_count = 0usize;
    for (page_num, bytes) in rendered_pages {
        let bytes = match bytes {
            Ok(bytes) => bytes,
            Err(err) => {
                eprintln!("Warning: skipped page {}: {}", page_num, err);
                continue;
            }
        };
        let filename = format!("page-{:03}.{}", page_num, format.file_extension());
        zip.start_file(&filename, zip_opts)?;
        zip.write_all(&bytes)?;
        rendered_count += 1;
    }
    zip.finish()?;

    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "op": "render",
                "output": args.output.display().to_string(),
                "format": args.format,
                "dpi": dpi,
                "pages_requested": page_nums.len(),
                "pages_rendered": rendered_count,
            })
        );
    } else {
        eprintln!(
            "Rendered {} page(s) at {} DPI -> {}",
            rendered_count,
            dpi,
            args.output.display()
        );
    }
    Ok(())
}

fn run_pdf_to_jpg(args: PdfToJpgArgs) -> Result<(), Box<dyn Error>> {
    let engine = open_engine(&args.pdf, &args.password)?;
    let total = engine.page_count()?;
    let pages = parse_page_range_cli(&args.pages, total)?;
    let format = wellfriendpdf_engine::RasterImageFormat::parse(&args.format)
        .ok_or_else(|| format!("unknown --format '{}'; use jpg or png", args.format))?;
    let results = wellfriendpdf_engine::export_pdf_pages_to_images(
        &engine,
        &args.out_dir,
        &pages,
        args.dpi,
        format,
        args.quality,
        &args.stem,
    )?;
    let failed = results.iter().filter(|r| !r.ok).count();
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "op": "pdf-to-jpg",
                "format": format.extension(),
                "pages": results,
                "ok": failed == 0,
                "failed_pages": failed,
            }))?
        );
    } else {
        eprintln!(
            "Rendered {} page(s) to {} ({} failure(s)).",
            results.len(),
            args.out_dir.display(),
            failed
        );
    }
    Ok(())
}

fn run_image_to_pdf(args: ImageToPdfArgs) -> Result<(), Box<dyn Error>> {
    let page_size =
        wellfriendpdf_engine::ImagePdfPageSize::parse(&args.page_size).ok_or_else(|| {
            format!(
                "unknown --page-size '{}'; use a4, letter, or size-to-image",
                args.page_size
            )
        })?;
    let bytes = wellfriendpdf_engine::images_to_pdf_from_paths(
        &args.images,
        wellfriendpdf_engine::ImageToPdfOptions {
            page_size,
            margin_points: args.margin,
        },
    )?;
    std::fs::write(&args.output, &bytes)?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "op": "image-to-pdf",
                "inputs": args.images,
                "output": args.output,
                "output_bytes": bytes.len(),
            }))?
        );
    } else {
        eprintln!(
            "Wrote {} page image(s) to {} ({} bytes).",
            args.images.len(),
            args.output.display(),
            bytes.len()
        );
    }
    Ok(())
}

fn run_pdf_to_xlsx(args: PdfToXlsxArgs) -> Result<(), Box<dyn Error>> {
    let engine = open_engine(&args.pdf, &args.password)?;
    let layout = wellfriendpdf_engine::XlsxLayout::parse(&args.layout)
        .ok_or_else(|| format!("unknown --layout '{}'; use pages or tables", args.layout))?;
    let bytes =
        wellfriendpdf_engine::pdf_to_xlsx(&engine, &wellfriendpdf_engine::XlsxOptions { layout })?;
    std::fs::write(&args.output, &bytes)?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "op": "pdf-to-xlsx",
                "input": args.pdf,
                "output": args.output,
                "layout": layout.as_str(),
                "output_bytes": bytes.len(),
            }))?
        );
    } else {
        eprintln!(
            "Wrote XLSX workbook to {} ({} bytes, layout {}).",
            args.output.display(),
            bytes.len(),
            layout.as_str()
        );
    }
    Ok(())
}

fn run_pdf_to_pptx(args: PdfToPptxArgs) -> Result<(), Box<dyn Error>> {
    let engine = open_engine(&args.pdf, &args.password)?;
    let options = wellfriendpdf_engine::PptxOptions {
        include_images: !args.no_images,
    };
    let bytes = wellfriendpdf_engine::pdf_to_pptx(&engine, &options)?;
    std::fs::write(&args.output, &bytes)?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "op": "pdf-to-pptx",
                "input": args.pdf,
                "output": args.output,
                "include_images": options.include_images,
                "output_bytes": bytes.len(),
            }))?
        );
    } else {
        eprintln!(
            "Wrote PPTX presentation to {} ({} bytes).",
            args.output.display(),
            bytes.len()
        );
    }
    Ok(())
}

fn run_pdf_to_docx(args: PdfToDocxArgs) -> Result<(), Box<dyn Error>> {
    let engine = open_engine(&args.pdf, &args.password)?;
    let layout = wellfriendpdf_engine::DocxLayout::parse(&args.layout)
        .ok_or_else(|| usage_error("unknown --layout; use flowing, page-faithful, or hybrid"))?;
    let options = wellfriendpdf_engine::DocxOptions {
        include_images: !args.no_images,
        layout,
    };
    let bytes = wellfriendpdf_engine::pdf_to_docx(&engine, &options)?;
    std::fs::write(&args.output, &bytes)?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "op": "pdf-to-docx",
                "input": args.pdf,
                "output": args.output,
                "include_images": options.include_images,
                "layout": options.layout.as_str(),
                "output_bytes": bytes.len(),
            }))?
        );
    } else {
        eprintln!(
            "Wrote DOCX document to {} ({} bytes).",
            args.output.display(),
            bytes.len()
        );
    }
    Ok(())
}

fn run_pdf_to_html(args: PdfToHtmlArgs) -> Result<(), Box<dyn Error>> {
    let engine = open_engine(&args.pdf, &args.password)?;
    let total = engine.page_count()?;
    let pages = parse_page_range_cli(&args.pages, total)?;
    let model = engine.build_editable_document(&wellfriendpdf_engine::EditableBuildOptions {
        pages: pages.clone(),
        ..wellfriendpdf_engine::EditableBuildOptions::default()
    })?;
    let output = model.to_semantic_html();
    std::fs::write(&args.output, output.as_bytes())?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "op": "pdf-to-html",
                "input": args.pdf,
                "output": args.output,
                "pages": pages,
                "blocks": model.blocks.len(),
                "output_bytes": output.len(),
            }))?
        );
    } else {
        eprintln!("Wrote semantic HTML to {}.", args.output.display());
    }
    Ok(())
}

fn run_pdf_to_markdown(args: PdfToMarkdownArgs) -> Result<(), Box<dyn Error>> {
    let engine = open_engine(&args.pdf, &args.password)?;
    let total = engine.page_count()?;
    let pages = parse_page_range_cli(&args.pages, total)?;
    let model = engine.build_editable_document(&wellfriendpdf_engine::EditableBuildOptions {
        pages: pages.clone(),
        ..wellfriendpdf_engine::EditableBuildOptions::default()
    })?;
    let output = model.to_markdown();
    std::fs::write(&args.output, output.as_bytes())?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "op": "pdf-to-markdown",
                "input": args.pdf,
                "output": args.output,
                "pages": pages,
                "blocks": model.blocks.len(),
                "output_bytes": output.len(),
            }))?
        );
    } else {
        eprintln!("Wrote Markdown to {}.", args.output.display());
    }
    Ok(())
}

fn run_pdf_to_json(args: PdfToJsonArgs) -> Result<(), Box<dyn Error>> {
    let engine = open_engine(&args.pdf, &args.password)?;
    let total = engine.page_count()?;
    let pages = parse_page_range_cli(&args.pages, total)?;
    let model = engine.build_editable_document(&wellfriendpdf_engine::EditableBuildOptions {
        pages: pages.clone(),
        ..wellfriendpdf_engine::EditableBuildOptions::default()
    })?;
    let output = serde_json::to_string_pretty(&model)?;
    std::fs::write(&args.output, output.as_bytes())?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "op": "pdf-to-json",
                "input": args.pdf,
                "output": args.output,
                "pages": pages,
                "blocks": model.blocks.len(),
                "output_bytes": output.len(),
            }))?
        );
    } else {
        eprintln!("Wrote editable JSON to {}.", args.output.display());
    }
    Ok(())
}

fn run_export_editable_model(args: ExportEditableModelArgs) -> Result<(), Box<dyn Error>> {
    let engine = open_engine(&args.pdf, &args.password)?;
    let total = engine.page_count()?;
    let pages = parse_page_range_cli(&args.pages, total)?;
    let model = engine.build_editable_document(&wellfriendpdf_engine::EditableBuildOptions {
        pages: pages.clone(),
        ..wellfriendpdf_engine::EditableBuildOptions::default()
    })?;
    let output = serde_json::to_string_pretty(&model)?;
    std::fs::write(&args.output, output.as_bytes())?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "op": "export-editable-model",
                "input": args.pdf,
                "output": args.output,
                "schema_version": model.schema_version,
                "pages": pages,
                "blocks": model.blocks.len(),
                "diagnostics": model.diagnostics.len(),
                "output_bytes": output.len(),
            }))?
        );
    } else {
        eprintln!("Wrote editable model JSON to {}.", args.output.display());
    }
    Ok(())
}

fn run_edit_text(args: EditTextArgs) -> Result<(), Box<dyn Error>> {
    let engine = open_engine(&args.pdf, &args.password)?;
    let total = engine.page_count()?;
    let pages = parse_page_range_cli(&args.pages, total)?;
    let rgb = parse_rgb_color(&args.color)?;
    let input = read_edit_input(&args.pdf, &args.password)?;
    if matches!(
        args.mode.as_str(),
        "rtl-reflow" | "vertical-reflow" | "same-width-patch"
    ) {
        if args.insert_at.is_some() || args.delete_range.is_some() {
            return Err(usage_error(
                "advanced editing rtl/vertical/same-width modes currently accept replacement operations only",
            ));
        }
        let page = *pages.first().ok_or_else(|| {
            usage_error("advanced editing edit requires at least one selected page")
        })?;
        let (bytes, report) = if args.mode == "same-width-patch" {
            let (bytes, report) = wellfriendpdf_engine::apply_same_width_patch(
                &input,
                page,
                &args.query,
                &args.replacement,
                &wellfriendpdf_engine::SameWidthPatchOptions {
                    signature_policy_override: args.signature_policy_override,
                    ..wellfriendpdf_engine::SameWidthPatchOptions::default()
                },
            )?;
            (bytes, serde_json::to_value(report)?)
        } else {
            let mode = if args.mode == "rtl-reflow" {
                wellfriendpdf_engine::AdvancedTextMode::ParagraphReflowRtl
            } else {
                wellfriendpdf_engine::AdvancedTextMode::ParagraphReflowVertical
            };
            let page_info = engine.document().get_page(page)?;
            let margin = 36.0;
            let (bytes, report) = wellfriendpdf_engine::edit_advanced_text_pdf(
                &input,
                page,
                &args.query,
                &args.replacement,
                mode,
                &wellfriendpdf_engine::AdvancedTextEditOptions {
                    region: [
                        page_info.crop_box[0] + margin,
                        page_info.crop_box[1] + margin,
                        page_info.crop_box[2] - margin,
                        page_info.crop_box[3] - margin,
                    ],
                    font_size: args.font_size,
                    signature_policy_override: args.signature_policy_override,
                    ..wellfriendpdf_engine::AdvancedTextEditOptions::default()
                },
                None,
            )?;
            (bytes, serde_json::to_value(report)?)
        };
        std::fs::write(&args.output, &bytes)?;
        if args.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "op": "edit-text",
                    "mode": args.mode,
                    "output": args.output,
                    "output_bytes": bytes.len(),
                    "report": report,
                }))?
            );
        } else {
            eprintln!("Edited text -> {}.", args.output.display());
        }
        return Ok(());
    }
    let style = wellfriendpdf_engine::EditTextStyle::new(args.font_size)
        .fill(wellfriendpdf_engine::Color::device_rgb(rgb.r, rgb.g, rgb.b));
    let mode = wellfriendpdf_engine::ParagraphEditSerializationMode::parse(&args.mode).ok_or_else(
        || usage_error("unknown --mode; use paragraph-reflow, safe-patch, or overlay-fallback"),
    )?;
    let operation = if let Some(range) = args.delete_range.as_deref() {
        let (start, end) = parse_char_range_cli(range)?;
        wellfriendpdf_engine::ParagraphEditOperation::Delete { start, end }
    } else if let Some(offset) = args.insert_at {
        wellfriendpdf_engine::ParagraphEditOperation::Insert {
            offset,
            text: args.replacement.clone(),
        }
    } else {
        wellfriendpdf_engine::ParagraphEditOperation::Replace {
            replacement: args.replacement.clone(),
        }
    };
    let (bytes, report) =
        if mode == wellfriendpdf_engine::ParagraphEditSerializationMode::OverlayFallback {
            let (bytes, report) = wellfriendpdf_engine::replace_text_pdf(
                input,
                &args.query,
                &args.replacement,
                wellfriendpdf_engine::TextReplacementOptions {
                    pages,
                    case_sensitive: !args.ignore_case,
                    max_replacements: args.max_replacements.max(1),
                    replacement_style: style,
                    ..wellfriendpdf_engine::TextReplacementOptions::default()
                },
            )?;
            (bytes, serde_json::to_value(report)?)
        } else {
            let (bytes, report) = wellfriendpdf_engine::edit_paragraph_reflow_pdf(
                input,
                &args.query,
                operation,
                wellfriendpdf_engine::ParagraphReflowOptions {
                    pages,
                    case_sensitive: !args.ignore_case,
                    max_edits: args.max_replacements.max(1),
                    replacement_style: style,
                    mode,
                    ..wellfriendpdf_engine::ParagraphReflowOptions::default()
                },
            )?;
            (bytes, serde_json::to_value(report)?)
        };
    std::fs::write(&args.output, &bytes)?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "op": "edit-text",
                "output": args.output,
                "output_bytes": bytes.len(),
                "report": report,
            }))?
        );
    } else {
        eprintln!("Edited text -> {}.", args.output.display());
    }
    Ok(())
}

fn run_save_incremental(args: SaveIncrementalArgs) -> Result<(), Box<dyn Error>> {
    let input = read_edit_input(&args.pdf, &args.password)?;
    let mut editor = wellfriendpdf_engine::PdfEditor::open_bytes(input.clone())?;
    editor.draw_text(
        args.page,
        args.text,
        args.x,
        args.y,
        wellfriendpdf_engine::EditTextStyle::new(args.font_size),
        wellfriendpdf_engine::OverlayLayer::Overlay,
    )?;
    let deterministic_options = wellfriendpdf_engine::DeterministicSaveOptions {
        fixed_pdf_date: args.fixed_timestamp.clone(),
        ..wellfriendpdf_engine::DeterministicSaveOptions::default()
    };
    let (bytes, report) = if args.deterministic || args.fixed_timestamp.is_some() {
        editor.save_to_bytes_with_options(
            wellfriendpdf_engine::EditMode::Incremental,
            &deterministic_options,
        )?
    } else {
        let bytes = editor.save_to_bytes(wellfriendpdf_engine::EditMode::Incremental)?;
        let output_bytes = bytes.len();
        (
            bytes,
            wellfriendpdf_engine::DeterministicSaveReport {
                mode: "incremental".to_string(),
                output_bytes,
                fixed_pdf_date: None,
                first_file_id_preserved: true,
                deterministic_resource_names: true,
                dedup_resources_requested: false,
                object_stream_packing: "incremental_plain_objects".to_string(),
                compression: "deterministic_flate_settings".to_string(),
                signature_invalidation_warning: false,
            },
        )
    };
    let original_prefix_preserved = bytes.starts_with(&input);
    std::fs::write(&args.output, &bytes)?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "op": "save-incremental",
                "input": args.pdf,
                "output": args.output,
                "page": args.page,
                "output_bytes": bytes.len(),
                "original_prefix_preserved": original_prefix_preserved,
                "deterministic": args.deterministic,
                "writer_report": report,
            }))?
        );
    } else {
        eprintln!(
            "Wrote incremental update to {} (prefix preserved: {}).",
            args.output.display(),
            original_prefix_preserved
        );
    }
    Ok(())
}

fn run_docx_to_pdf(args: OfficeToPdfArgs) -> Result<(), Box<dyn Error>> {
    run_office_to_pdf(args, "docx-to-pdf", wellfriendpdf_engine::docx_to_pdf)
}

fn run_xlsx_to_pdf(args: OfficeToPdfArgs) -> Result<(), Box<dyn Error>> {
    run_office_to_pdf(args, "xlsx-to-pdf", wellfriendpdf_engine::xlsx_to_pdf)
}

fn run_pptx_to_pdf(args: OfficeToPdfArgs) -> Result<(), Box<dyn Error>> {
    run_office_to_pdf(args, "pptx-to-pdf", wellfriendpdf_engine::pptx_to_pdf)
}

fn run_office_to_pdf(
    args: OfficeToPdfArgs,
    op: &str,
    convert: fn(
        &[u8],
        &wellfriendpdf_engine::OfficeToPdfOptions,
    ) -> wellfriendpdf_engine::Result<Vec<u8>>,
) -> Result<(), Box<dyn Error>> {
    let input = std::fs::read(&args.input)?;
    let bytes = convert(&input, &wellfriendpdf_engine::OfficeToPdfOptions::default())?;
    std::fs::write(&args.output, &bytes)?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "op": op,
                "input": args.input,
                "output": args.output,
                "output_bytes": bytes.len(),
            }))?
        );
    } else {
        eprintln!(
            "Wrote PDF to {} ({} bytes).",
            args.output.display(),
            bytes.len()
        );
    }
    Ok(())
}

fn run_render_svg(args: RenderArgs, dpi: u32) -> Result<(), Box<dyn Error>> {
    use std::io::Write;
    use zip::{write::FileOptions, CompressionMethod, ZipWriter};

    let engine = open_engine(&args.pdf, &args.password)?;
    let total = engine.page_count()?;
    let page_nums = parse_page_range_cli(&args.pages, total)?;

    let out_file = std::fs::File::create(&args.output)?;
    let mut zip = ZipWriter::new(out_file);
    let zip_opts = FileOptions::<()>::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(6));

    let mut rendered = 0usize;
    let mut rasterized_fallback = 0usize;
    for page_num in &page_nums {
        let page = match engine.render_page_svg(*page_num, dpi) {
            Ok(page) => page,
            Err(err) => {
                eprintln!("Warning: skipped page {}: {}", page_num, err);
                continue;
            }
        };
        if page.is_rasterized {
            rasterized_fallback += 1;
        }
        let filename = format!("page-{:03}.svg", page_num);
        zip.start_file(&filename, zip_opts)?;
        zip.write_all(page.svg.as_bytes())?;
        rendered += 1;
    }
    zip.finish()?;

    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "op": "render",
                "output": args.output.display().to_string(),
                "format": "svg",
                "dpi": dpi,
                "pages_requested": page_nums.len(),
                "pages_rendered": rendered,
                "rasterized_fallback_pages": rasterized_fallback,
            })
        );
    } else {
        eprintln!(
            "Rendered {} page(s) to SVG -> {} ({} page(s) used the raster-embed fallback)",
            rendered,
            args.output.display(),
            rasterized_fallback
        );
    }
    Ok(())
}

/// PostScript output (`pdftops` / `pdftocairo -ps` equivalent): a single
/// DSC-conformant multi-page `.ps` document written directly to `--output`.
fn run_render_ps(args: RenderArgs, dpi: u32) -> Result<(), Box<dyn Error>> {
    use std::io::Write;

    let engine = open_engine(&args.pdf, &args.password)?;
    let total = engine.page_count()?;
    let page_nums = parse_page_range_cli(&args.pages, total)?;

    let (ps, rasterized) = engine.render_document_ps(&page_nums, dpi)?;

    // A single .ps document is the natural PostScript artifact (unlike the
    // per-page raster/SVG ZIP). If the output path still ends in .zip (the
    // default), retarget it to .ps so users get a usable file.
    let out_path = if args
        .output
        .extension()
        .map(|e| e.eq_ignore_ascii_case("zip"))
        .unwrap_or(false)
    {
        args.output.with_extension("ps")
    } else {
        args.output.clone()
    };

    let mut file = std::fs::File::create(&out_path)?;
    file.write_all(ps.as_bytes())?;

    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "op": "render",
                "output": out_path.display().to_string(),
                "format": "ps",
                "dpi": dpi,
                "pages_requested": page_nums.len(),
                "pages_rendered": page_nums.len(),
                "rasterized_fallback_pages": rasterized,
            })
        );
    } else {
        eprintln!(
            "Rendered {} page(s) to PostScript -> {} ({} page(s) used the raster-embed fallback)",
            page_nums.len(),
            out_path.display(),
            rasterized
        );
    }
    Ok(())
}

/// EPS output (`pdftops -eps` / `pdftocairo -eps` equivalent): one
/// single-page, EPSF-conformant `.eps` per page inside the output ZIP (EPS is
/// single-page by definition).
fn run_render_eps(args: RenderArgs, dpi: u32) -> Result<(), Box<dyn Error>> {
    use std::io::Write;
    use zip::{write::FileOptions, CompressionMethod, ZipWriter};

    let engine = open_engine(&args.pdf, &args.password)?;
    let total = engine.page_count()?;
    let page_nums = parse_page_range_cli(&args.pages, total)?;

    let out_file = std::fs::File::create(&args.output)?;
    let mut zip = ZipWriter::new(out_file);
    let zip_opts = FileOptions::<()>::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(6));

    let mut rendered = 0usize;
    let mut rasterized_fallback = 0usize;
    for page_num in &page_nums {
        let (eps, rasterized) = match engine.render_page_eps(*page_num, dpi) {
            Ok(v) => v,
            Err(err) => {
                eprintln!("Warning: skipped page {}: {}", page_num, err);
                continue;
            }
        };
        if rasterized {
            rasterized_fallback += 1;
        }
        let filename = format!("page-{:03}.eps", page_num);
        zip.start_file(&filename, zip_opts)?;
        zip.write_all(eps.as_bytes())?;
        rendered += 1;
    }
    zip.finish()?;

    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "op": "render",
                "output": args.output.display().to_string(),
                "format": "eps",
                "dpi": dpi,
                "pages_requested": page_nums.len(),
                "pages_rendered": rendered,
                "rasterized_fallback_pages": rasterized_fallback,
            })
        );
    } else {
        eprintln!(
            "Rendered {} page(s) to EPS -> {} ({} page(s) used the raster-embed fallback)",
            rendered,
            args.output.display(),
            rasterized_fallback
        );
    }
    Ok(())
}

fn run_analyze(args: AnalyzeArgs) -> Result<(), Box<dyn Error>> {
    use wellfriendpdf_engine::{ContentEngine, PdfAnalyzer};

    let engine = ContentEngine::open_path(&args.pdf)?;
    let analysis = PdfAnalyzer::quick_analysis(&engine)?;

    let json = if args.pretty {
        serde_json::to_string_pretty(&serde_json::json!({
            "has_text_layer": analysis.has_text_layer,
            "confidence": analysis.confidence,
            "pages_with_text": analysis.pages_with_text,
            "is_likely_scanned": analysis.is_likely_scanned,
            "recommendation": analysis.recommendation,
        }))?
    } else {
        serde_json::to_string(&serde_json::json!({
            "has_text_layer": analysis.has_text_layer,
            "confidence": analysis.confidence,
            "is_likely_scanned": analysis.is_likely_scanned,
        }))?
    };

    println!("{}", json);
    Ok(())
}

fn open_engine(
    pdf: &std::path::Path,
    password: &Option<String>,
) -> Result<wellfriendpdf_engine::ContentEngine, Box<dyn Error>> {
    use wellfriendpdf_engine::ContentEngine;
    let engine = match password {
        Some(pw) => ContentEngine::open_path_with_password(pdf, pw.as_bytes())?,
        None => ContentEngine::open_path(pdf)?,
    };
    Ok(engine)
}

fn write_output_optional(output: &Option<PathBuf>, text: &str) -> Result<(), Box<dyn Error>> {
    match output {
        Some(path) => std::fs::write(path, text)?,
        None => println!("{text}"),
    }
    Ok(())
}

fn read_edit_input(
    pdf: &std::path::Path,
    password: &Option<String>,
) -> Result<Vec<u8>, Box<dyn Error>> {
    if password.is_some() {
        let engine = open_engine(pdf, password)?;
        Ok(wellfriendpdf_engine::decrypt_pdf(&engine)?)
    } else {
        Ok(std::fs::read(pdf)?)
    }
}

fn parse_stamp_position(
    value: &str,
) -> Result<wellfriendpdf_engine::StampPosition, Box<dyn Error>> {
    wellfriendpdf_engine::StampPosition::parse(value)
        .ok_or_else(|| format!("unknown position '{value}'").into())
}

fn parse_rgb_color(value: &str) -> Result<wellfriendpdf_engine::RgbColor, Box<dyn Error>> {
    let hex = value.strip_prefix('#').unwrap_or(value);
    if hex.len() != 6 {
        return Err(format!("color '{value}' must be #RRGGBB").into());
    }
    let r = u8::from_str_radix(&hex[0..2], 16)?;
    let g = u8::from_str_radix(&hex[2..4], 16)?;
    let b = u8::from_str_radix(&hex[4..6], 16)?;
    Ok(wellfriendpdf_engine::RgbColor {
        r: f64::from(r) / 255.0,
        g: f64::from(g) / 255.0,
        b: f64::from(b) / 255.0,
    })
}

#[derive(Debug, Clone, Copy)]
struct RedactRegionSpec {
    page: usize,
    rect: wellfriendpdf_engine::ImageRect,
}

fn parse_redact_rect_cli(
    spec: &str,
    total_pages: usize,
) -> Result<RedactRegionSpec, Box<dyn Error>> {
    let (page_str, rect_str) = spec
        .split_once(':')
        .ok_or_else(|| usage_error("redaction rect must be page:x,y,w,h"))?;
    let page = page_str.trim().parse::<usize>()?;
    if !(1..=total_pages).contains(&page) {
        return Err(usage_error(format!(
            "redaction rect page {page} is out of range 1..={total_pages}"
        )));
    }
    let values: Vec<f64> = rect_str
        .split(',')
        .map(|part| part.trim().parse::<f64>())
        .collect::<Result<Vec<_>, _>>()?;
    if values.len() != 4 {
        return Err(usage_error("redaction rect must be page:x,y,w,h"));
    }
    if values.iter().any(|v| !v.is_finite()) || values[2] <= 0.0 || values[3] <= 0.0 {
        return Err(usage_error(
            "redaction rect coordinates must be finite and width/height must be positive",
        ));
    }
    Ok(RedactRegionSpec {
        page,
        rect: wellfriendpdf_engine::ImageRect::new(values[0], values[1], values[2], values[3]),
    })
}

fn parse_plain_rect_cli(spec: &str) -> Result<wellfriendpdf_engine::ImageRect, Box<dyn Error>> {
    let values: Vec<f64> = spec
        .split(',')
        .map(|part| part.trim().parse::<f64>())
        .collect::<Result<Vec<_>, _>>()?;
    if values.len() != 4 {
        return Err(usage_error("rectangle must be x,y,w,h"));
    }
    if values.iter().any(|v| !v.is_finite()) || values[2] <= 0.0 || values[3] <= 0.0 {
        return Err(usage_error(
            "rectangle coordinates must be finite and width/height must be positive",
        ));
    }
    Ok(wellfriendpdf_engine::ImageRect::new(
        values[0], values[1], values[2], values[3],
    ))
}

fn parse_form_data_format(
    value: &str,
) -> Result<wellfriendpdf_engine::FormDataFormat, Box<dyn Error>> {
    wellfriendpdf_engine::FormDataFormat::parse(value)
        .ok_or_else(|| usage_error(format!("unknown form data format '{value}'")))
}

fn parse_image_redaction_policy(
    value: &str,
) -> Result<wellfriendpdf_engine::ImageRedactionPolicy, Box<dyn Error>> {
    match value.to_ascii_lowercase().as_str() {
        "partial" => Ok(wellfriendpdf_engine::ImageRedactionPolicy::Partial),
        "remove" => Ok(wellfriendpdf_engine::ImageRedactionPolicy::Remove),
        "fail" => Ok(wellfriendpdf_engine::ImageRedactionPolicy::Fail),
        _ => Err(usage_error(format!(
            "unknown --image-policy '{value}'; expected partial, remove, or fail"
        ))),
    }
}

fn parse_attachment_policy(
    value: &str,
) -> Result<wellfriendpdf_engine::AttachmentRedactionPolicy, Box<dyn Error>> {
    match value.to_ascii_lowercase().as_str() {
        "keep" => Ok(wellfriendpdf_engine::AttachmentRedactionPolicy::Keep),
        "remove-all" | "remove_all" | "all" => {
            Ok(wellfriendpdf_engine::AttachmentRedactionPolicy::RemoveAll)
        }
        "remove-overlapping" | "remove_overlapping" | "overlapping" => {
            Ok(wellfriendpdf_engine::AttachmentRedactionPolicy::RemoveOverlapping)
        }
        _ => Err(usage_error(format!(
            "unknown --attachments '{value}'; expected keep, remove-all, or remove-overlapping"
        ))),
    }
}

fn redaction_rect_from_quads(
    quads: &[wellfriendpdf_engine::TextQuad],
) -> Option<wellfriendpdf_engine::ImageRect> {
    let bbox = wellfriendpdf_engine::TextQuad::union(quads)?;
    let pad = 0.5;
    Some(wellfriendpdf_engine::ImageRect::new(
        bbox.x0 - pad,
        bbox.y0 - pad,
        (bbox.x1 - bbox.x0 + pad * 2.0).max(0.1),
        (bbox.y1 - bbox.y0 + pad * 2.0).max(0.1),
    ))
}

fn run_info(args: InfoArgs) -> Result<(), Box<dyn Error>> {
    let engine = open_engine(&args.pdf, &args.password)?;
    let info = engine.document_info()?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&info)?);
        return Ok(());
    }

    // Human-readable, pdfinfo-style "Label: value" lines. Optional fields are
    // only printed when present.
    let print_field = |label: &str, value: &str| {
        if !value.is_empty() {
            println!("{label:<16} {value}");
        }
    };
    print_field("Title:", info.title.as_deref().unwrap_or(""));
    print_field("Subject:", info.subject.as_deref().unwrap_or(""));
    print_field("Keywords:", info.keywords.as_deref().unwrap_or(""));
    print_field("Author:", info.author.as_deref().unwrap_or(""));
    print_field("Creator:", info.creator.as_deref().unwrap_or(""));
    print_field("Producer:", info.producer.as_deref().unwrap_or(""));
    print_field("CreationDate:", info.creation_date.as_deref().unwrap_or(""));
    print_field("ModDate:", info.mod_date.as_deref().unwrap_or(""));

    println!("{:<16} {}", "Tagged:", yes_no(info.tagged));
    println!("{:<16} {}", "Pages:", info.page_count);

    // Page size: first size, or "varies" with the distinct list.
    if let Some(first) = info.page_sizes.first() {
        let label = page_size_label(first);
        if info.page_size_varies {
            println!("{:<16} varies", "Page size:");
            for s in &info.page_sizes {
                println!(
                    "                 {} ({} page(s))",
                    page_size_label(s),
                    s.page_count
                );
            }
        } else {
            println!("{:<16} {}", "Page size:", label);
        }
    }

    println!("{:<16} {}", "Encrypted:", yes_no(info.encrypted));
    if let Some(enc) = &info.encryption {
        println!(
            "{:<16} {} (V{} R{}, {}-bit)",
            "  Algorithm:", enc.algorithm, enc.version, enc.revision, enc.key_length_bits
        );
        let p = &enc.permissions;
        println!(
            "{:<16} print:{} copy:{} modify:{} annotate:{} fill:{} accessible:{} assemble:{} hq-print:{}",
            "  Permissions:",
            yes_no(p.print),
            yes_no(p.copy),
            yes_no(p.modify),
            yes_no(p.annotate),
            yes_no(p.fill_forms),
            yes_no(p.extract_accessibility),
            yes_no(p.assemble),
            yes_no(p.high_quality_print),
        );
    }

    println!("{:<16} {}", "Optimized:", yes_no(info.linearized));
    println!("{:<16} {}", "PDF version:", info.pdf_version);
    println!("{:<16} {} bytes", "File size:", info.file_size_bytes);
    if let Some(id) = &info.file_id {
        println!("{:<16} {}", "File ID:", id);
    }
    println!("{:<16} {}", "XMP Metadata:", yes_no(info.has_xmp_metadata));

    Ok(())
}

fn run_feature_report(args: FeatureReportArgs) -> Result<(), Box<dyn Error>> {
    let json = wellfriendpdf_engine::sdk::feature_report_json()?;
    let output = if args.pretty {
        let value: serde_json::Value = serde_json::from_str(&json)?;
        serde_json::to_string_pretty(&value)?
    } else {
        json
    };
    write_output_optional(&args.output, &output)
}

fn run_parser_report(args: ParserReportArgs) -> Result<(), Box<dyn Error>> {
    let mode = match args.mode.to_ascii_lowercase().as_str() {
        "strict" => wellfriendpdf_engine::ParserMode::Strict,
        "repair" => wellfriendpdf_engine::ParserMode::Repair,
        "audit" => wellfriendpdf_engine::ParserMode::Audit,
        other => {
            return Err(format!(
                "unknown parser report mode '{other}'; use strict, repair, or audit"
            )
            .into());
        }
    };
    let bytes = std::fs::read(&args.pdf)?;
    let password = args.password.as_deref().unwrap_or("").as_bytes();
    let decode_limits = parser_report_decode_limits(&args)?;
    let mut report = wellfriendpdf_engine::parser_report_bytes_with_options(
        &bytes,
        mode,
        password,
        wellfriendpdf_engine::ParserReportOptions {
            include_decode: args.include_decode,
            decode_limits,
        },
    );
    if let Some(max) = args.max_diagnostics {
        report.diagnostics.truncate(max);
    }
    let color_report = if args.include_color {
        Some(wellfriendpdf_engine::color_report_bytes(
            &bytes,
            parser_report_color_profile(&args.color_profile)?,
        )?)
    } else {
        None
    };
    let _include_flags = (
        args.include_revisions,
        args.include_arlington,
        args.include_linearization,
        args.include_repair,
        args.include_source_metrics,
        args.include_decode,
        args.include_color,
    );
    let output = if args.pretty && !args.json {
        let mut human = format_parser_report_human(&args.pdf, &report);
        if let Some(color) = &color_report {
            human.push_str(&format!(
                "\nColor spaces: {} families\nOutput intents: {}\nColor diagnostics: {}\n",
                color.color_spaces.len(),
                color.output_intents.len(),
                color.diagnostics.len()
            ));
        }
        human
    } else if let Some(color) = color_report {
        let mut value = serde_json::to_value(&report)?;
        if let Some(obj) = value.as_object_mut() {
            obj.insert("color".to_string(), serde_json::to_value(color)?);
        }
        serde_json::to_string_pretty(&value)?
    } else {
        serde_json::to_string_pretty(&report)?
    };
    match args.output {
        Some(path) => std::fs::write(path, output)?,
        None => println!("{output}"),
    }
    enforce_parser_report_fail_on(&args.fail_on, &report)?;
    Ok(())
}

fn parser_report_color_profile(
    profile: &str,
) -> Result<wellfriendpdf_engine::ColorValidationProfile, Box<dyn Error>> {
    match profile.to_ascii_lowercase().as_str() {
        "generic" | "default" => Ok(wellfriendpdf_engine::ColorValidationProfile::Generic),
        "pdfa" | "pdf/a" | "pdf-a" => Ok(wellfriendpdf_engine::ColorValidationProfile::PdfA),
        "pdfx" | "pdf/x" | "pdf-x" => Ok(wellfriendpdf_engine::ColorValidationProfile::PdfX),
        other => Err(format!(
            "unknown --color-profile value '{other}'; use generic, pdfa, or pdfx"
        )
        .into()),
    }
}

fn parser_report_decode_limits(
    args: &ParserReportArgs,
) -> Result<wellfriendpdf_engine::DecodeLimits, Box<dyn Error>> {
    let mut limits = match args.decode_profile.to_ascii_lowercase().as_str() {
        "default" => wellfriendpdf_engine::DecodeLimits::default(),
        "low-memory" | "low_memory" => wellfriendpdf_engine::DecodeLimits::strict_low_memory(),
        "audit" => wellfriendpdf_engine::DecodeLimits::audit_generous(),
        other => {
            return Err(format!(
                "unknown --decode-profile value '{other}'; use default, low-memory, or audit"
            )
            .into())
        }
    };
    if let Some(mb) = args.decode_max_stream_mb {
        limits.max_decoded_bytes_per_stream = mb
            .checked_mul(1024 * 1024)
            .ok_or("--decode-max-stream-mb overflows u64")?;
    }
    if let Some(depth) = args.decode_max_chain_depth {
        limits.max_filter_chain_depth = depth;
    }
    if let Some(mpixels) = args.decode_max_image_mpixels {
        limits.max_image_pixels = mpixels
            .checked_mul(1_000_000)
            .ok_or("--decode-max-image-mpixels overflows u64")?;
    }
    if let Some(mb) = args.decode_cache_mb {
        limits.cache_budget_bytes = usize::try_from(
            mb.checked_mul(1024 * 1024)
                .ok_or("--decode-cache-mb overflows u64")?,
        )?;
    }
    if let Some(mb) = args.decode_scheduler_mb {
        limits.scheduler_memory_budget_bytes = mb
            .checked_mul(1024 * 1024)
            .ok_or("--decode-scheduler-mb overflows u64")?;
    }
    Ok(limits)
}

fn enforce_parser_report_fail_on(
    fail_on: &str,
    report: &wellfriendpdf_engine::ParserReport,
) -> Result<(), Box<dyn Error>> {
    let should_fail = match fail_on.to_ascii_lowercase().as_str() {
        "never" => false,
        "fatal" => report.diagnostics.iter().any(|diagnostic| {
            matches!(
                diagnostic.severity,
                wellfriendpdf_engine::ParserSeverity::FatalError
                    | wellfriendpdf_engine::ParserSeverity::SecurityLimit
            )
        }),
        "error" => report.diagnostics.iter().any(|diagnostic| {
            matches!(
                diagnostic.severity,
                wellfriendpdf_engine::ParserSeverity::RecoverableError
                    | wellfriendpdf_engine::ParserSeverity::FatalError
                    | wellfriendpdf_engine::ParserSeverity::SecurityLimit
            )
        }),
        other => {
            return Err(
                format!("unknown --fail-on value '{other}'; use error, fatal, or never").into(),
            )
        }
    };
    if should_fail {
        Err("parser-report diagnostics reached the requested --fail-on severity".into())
    } else {
        Ok(())
    }
}

fn format_parser_report_human(path: &Path, report: &wellfriendpdf_engine::ParserReport) -> String {
    let counts = report.diagnostic_counts();
    let verdict = if report.opened { "opened" } else { "failed" };
    let mut out = String::new();
    out.push_str(&format!("Parser report for {}\n", path.display()));
    out.push_str(&format!("Verdict: {verdict} ({:?})\n", report.mode));
    out.push_str(&format!(
        "Diagnostics: info={} warning={} recoverable_error={} fatal={} security_limit={}\n",
        counts.info,
        counts.warning,
        counts.recoverable_error,
        counts.fatal_error,
        counts.security_limit
    ));
    out.push_str(&format!(
        "Source: {} bytes, {} known object(s), {} xref entrie(s)\n",
        report.source_metrics.file_size_bytes,
        report.source_metrics.objects_known,
        report.source_metrics.xref_entries
    ));
    out.push_str(&format!(
        "Linearized: detected={} valid={} fast-open-candidate={} status={}\n",
        report.linearization.is_linearized,
        report.linearization.valid,
        report.linearization.first_page_fast_open_candidate,
        report.linearization.main_xref_status
    ));
    out.push_str(&format!(
        "Revisions: {} section(s), incremental={}\n",
        report.revision_history.section_count, report.revision_history.contains_incremental_updates
    ));
    out.push_str(&format!(
        "Repair: xref_objects={} scan_objects={} duplicate_objects={} truncated_objects={} confidence={}\n",
        report.repair_summary.total_objects_recovered_from_xref,
        report.repair_summary.total_objects_recovered_from_scan,
        report.repair_summary.duplicate_objects.len(),
        report.repair_summary.truncated_objects.len(),
        report.repair_summary.confidence
    ));
    out.push_str(&format!(
        "Arlington: {} rules from {} TSV files at {}\n",
        report.arlington.keys, report.arlington.tsv_files, report.arlington.commit
    ));
    if let Some(decode) = &report.decode {
        out.push_str(&format!(
            "Decode: streams_seen={} decoded={} failed={} unsupported={} raw_bytes={} decoded_bytes={}\n",
            decode.metrics.streams_seen,
            decode.metrics.streams_decoded,
            decode.metrics.streams_failed,
            decode.metrics.unsupported_filters,
            decode.metrics.total_raw_bytes,
            decode.metrics.total_decoded_bytes
        ));
    }
    if !report.diagnostics.is_empty() {
        out.push_str("Top diagnostics:\n");
        for diagnostic in report.diagnostics.iter().take(20) {
            out.push_str(&format!(
                "- {:?}/{:?} {}: {}\n",
                diagnostic.severity, diagnostic.category, diagnostic.code, diagnostic.message
            ));
        }
    }
    out
}

fn page_size_label(s: &wellfriendpdf_engine::PageSize) -> String {
    let base = format!(
        "{:.2} x {:.2} pts ({:.0} x {:.0} mm)",
        s.width_pts,
        s.height_pts,
        s.width_pts * 25.4 / 72.0,
        s.height_pts * 25.4 / 72.0,
    );
    if s.rotation != 0 {
        format!("{base} rotated {}\u{00B0}", s.rotation)
    } else {
        base
    }
}

fn run_fonts(args: FontsArgs) -> Result<(), Box<dyn Error>> {
    let engine = open_engine(&args.pdf, &args.password)?;
    let fonts = engine.list_fonts()?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&fonts)?);
        return Ok(());
    }

    if fonts.is_empty() {
        println!("(no fonts found)");
        return Ok(());
    }

    // pdffonts-style table.
    println!(
        "{:<32} {:<16} {:<16} {:>3} {:>3} {:>3} {:>8}",
        "name", "type", "encoding", "emb", "sub", "uni", "object ID"
    );
    println!("{}", "-".repeat(88));
    for f in &fonts {
        println!(
            "{:<32} {:<16} {:<16} {:>3} {:>3} {:>3} {:>5} {:>1}",
            truncate(&f.name, 32),
            truncate(&f.font_type, 16),
            truncate(&f.encoding, 16),
            yes_no(f.embedded),
            yes_no(f.subset),
            yes_no(f.to_unicode),
            f.object_number,
            f.generation,
        );
    }

    Ok(())
}

fn yes_no(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        "no"
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max.saturating_sub(1)])
    }
}

fn run_detach(args: DetachArgs) -> Result<(), Box<dyn Error>> {
    use wellfriendpdf_engine::sanitize_filename;

    let engine = open_engine(&args.pdf, &args.password)?;
    let attachments = engine.list_attachments()?;

    let want_save = args.save.is_some() || args.name.is_some() || args.save_all;

    // Default action (and explicit --list) is to list.
    if !want_save || args.list {
        if args.json {
            println!("{}", serde_json::to_string_pretty(&attachments)?);
        } else if attachments.is_empty() {
            println!("0 embedded files");
        } else {
            println!("{} embedded file(s)", attachments.len());
            for a in &attachments {
                let size = a
                    .size
                    .map(|s| format!("{s} bytes"))
                    .unwrap_or_else(|| "size unknown".to_string());
                let desc = a
                    .description
                    .as_deref()
                    .map(|d| format!(" — {d}"))
                    .unwrap_or_default();
                println!("{}: {} ({}){}", a.index, a.name, size, desc);
            }
        }
        // If only listing was requested, stop here.
        if !want_save {
            return Ok(());
        }
    }

    // Determine which attachments to save.
    let to_save: Vec<&wellfriendpdf_engine::Attachment> = if args.save_all {
        attachments.iter().collect()
    } else if let Some(n) = args.save {
        let a = attachments
            .iter()
            .find(|a| a.index == n)
            .ok_or_else(|| format!("no attachment with index {n} (have {})", attachments.len()))?;
        vec![a]
    } else if let Some(name) = &args.name {
        let a = attachments
            .iter()
            .find(|a| &a.name == name)
            .ok_or_else(|| format!("no attachment named '{name}'"))?;
        vec![a]
    } else {
        Vec::new()
    };

    if to_save.is_empty() {
        return Ok(());
    }

    std::fs::create_dir_all(&args.output_dir)?;
    for a in to_save {
        let bytes = engine.extract_attachment(a)?;
        // Sanitize the attacker-controlled name down to a single safe
        // component, then join onto the chosen output directory.
        let safe = sanitize_filename(&a.name);
        let target = args.output_dir.join(&safe);
        std::fs::write(&target, &bytes)?;
        eprintln!(
            "Saved attachment {} '{}' -> {} ({} bytes)",
            a.index,
            a.name,
            target.display(),
            bytes.len()
        );
    }

    Ok(())
}

fn run_to_html(args: ToHtmlArgs) -> Result<(), Box<dyn Error>> {
    use wellfriendpdf_engine::{HtmlMode, HtmlOptions};

    let engine = open_engine(&args.pdf, &args.password)?;
    let total = engine.page_count()?;
    let page_nums = parse_page_range_cli(&args.pages, total)?;
    if page_nums.is_empty() {
        return Err("no pages selected".into());
    }

    // Mode precedence: --xml, then --simple, else complex (the default).
    let mode = if args.xml {
        HtmlMode::Xml
    } else if args.simple {
        HtmlMode::Simple
    } else {
        HtmlMode::Complex
    };
    if args.complex && (args.simple || args.xml) {
        eprintln!("Warning: --complex ignored because --simple/--xml was given");
    }

    let options = HtmlOptions {
        mode,
        background: args.background,
        background_dpi: args.background_dpi.clamp(24, 600),
        invisible_text_over_background: args.invisible_text,
        ..Default::default()
    };

    let doc = engine.export_html(&page_nums, &options)?;

    match args.output {
        Some(path) => {
            std::fs::write(&path, doc.as_bytes())?;
            eprintln!(
                "Wrote {} page(s) as {:?} -> {}",
                page_nums.len(),
                mode,
                path.display()
            );
        }
        None => print!("{doc}"),
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SignatureCliMode {
    /// Historical `verify-sig` behavior remains an inspection command so
    /// existing scripts can inventory mathematically valid but untrusted PDFs.
    LegacyInspect,
    /// `signature-list` reports inventory only and does not assert policy.
    List,
    SignatureVerify,
    PadesVerify,
    SignatureTimestamps,
    DssInspect,
    LtvVerify,
    PadesLevelReport,
    CertificatePathBuild,
    CertificatePathVerify,
    OcspCheck,
    CrlCheck,
    RevocationCheck,
}

fn run_verify_sig(args: VerifySigArgs, mode: SignatureCliMode) -> Result<(), Box<dyn Error>> {
    use wellfriendpdf_engine::{
        Coverage, PadesLevel, RevocationStatus, SignatureStatus, SignatureTrust, SignatureValidity,
    };

    let engine = open_engine(&args.pdf, &args.password)?;
    let options = verify_options_from_args(&args)?;
    let reports = if let Some(path) = &args.evidence_out {
        let outcome = engine.verify_signatures_with_options_and_evidence(&options)?;
        write_evidence_bundle(path, &outcome.evidence_bundle, &options)?;
        outcome.reports
    } else {
        engine.verify_signatures_with_options(&options)?
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&reports)?);
        return finish_signature_cli_validation(&reports, mode);
    }

    if reports.is_empty() {
        println!("No digital signatures found.");
        return finish_signature_cli_validation(&reports, mode);
    }

    println!("{} signature(s) found.\n", reports.len());
    for r in &reports {
        let verdict = match r.validity {
            SignatureValidity::Valid => "Signature is cryptographically VALID",
            SignatureValidity::Invalid => "Signature is INVALID (digest/signature mismatch)",
            SignatureValidity::UnsupportedAlgorithm => "Signature algorithm UNSUPPORTED",
            SignatureValidity::Error => "Signature could NOT be verified",
        };
        let overall = match r.status {
            SignatureStatus::Trusted => "TRUSTED (integrity + trusted chain + whole-file coverage)",
            SignatureStatus::ValidUntrusted => {
                "VALID but UNTRUSTED (cryptographically valid; signer not trusted)"
            }
            SignatureStatus::Revoked => "REVOKED (validated revocation evidence)",
            SignatureStatus::ValidButModified => "VALID but document MODIFIED after signing",
            SignatureStatus::Invalid => "INVALID",
            SignatureStatus::UnsupportedAlgorithm => "UNSUPPORTED algorithm",
            SignatureStatus::Error => "could NOT be verified",
        };
        let trust = match r.trust {
            SignatureTrust::NotVerified => {
                "not verified (no trust anchors configured — pass anchors to evaluate trust)"
            }
            SignatureTrust::Trusted => "trusted (chains to a configured anchor)",
            SignatureTrust::Untrusted => "UNTRUSTED (self-signed or unknown issuer)",
            SignatureTrust::Expired => "signer certificate EXPIRED / not yet valid",
            SignatureTrust::Revoked => "signer certificate REVOKED",
        };
        let coverage = match r.coverage {
            Coverage::WholeFile => "covers the whole file",
            Coverage::ModifiedAfterSigning => "document MODIFIED after signing (bytes appended)",
        };
        println!("Signature #{}:", r.index);
        if let Some(f) = &r.field_name {
            println!("  - Field:        {f}");
        }
        if let Some(n) = &r.signer_name {
            println!("  - Signer:       {n}");
        }
        if let Some(t) = &r.signing_time {
            println!("  - Signing time: {t}");
        }
        if let Some(s) = &r.sub_filter {
            println!("  - SubFilter:    {s}");
        }
        if let Some(d) = &r.digest_algorithm {
            println!("  - Digest:       {d}");
        }
        if let Some(reason) = &r.reason {
            println!("  - Reason:       {reason}");
        }
        if let Some(loc) = &r.location {
            println!("  - Location:     {loc}");
        }
        println!("  - Status:       {overall}");
        println!("  - Integrity:    {verdict}");
        println!("  - Trust:        {trust}");
        println!("  - Coverage:     {coverage}");
        let pades = match r.ltv.pades_level {
            PadesLevel::BaselineB => "PAdES B-B / core CMS",
            PadesLevel::BaselineT => "PAdES B-T timestamped",
            PadesLevel::BaselineLT => "PAdES B-LT long-term material",
            PadesLevel::BaselineLTA => "PAdES B-LTA archive timestamp",
        };
        let revocation = match r.ltv.revocation_status {
            RevocationStatus::NotChecked => "not checked",
            RevocationStatus::EmbeddedMaterial => "embedded material present",
            RevocationStatus::GoodFromEmbeddedCrl => "not listed in embedded CRL",
            RevocationStatus::RevokedByEmbeddedCrl => "revoked by embedded CRL",
            RevocationStatus::Unknown => "unknown",
        };
        println!("  - PAdES/LTV:    {pades}");
        println!(
            "      Timestamp tokens: {} valid, {} invalid",
            r.ltv.timestamp_token_count, r.ltv.invalid_timestamp_token_count
        );
        println!(
            "      DSS/VRI: {} / {}",
            if r.ltv.dss_present {
                "present"
            } else {
                "absent"
            },
            if r.ltv.vri_matched {
                "matched"
            } else {
                "not matched"
            }
        );
        println!(
            "      Material: certs={}, ocsp={}, crls={}, revocation={}",
            r.ltv.embedded_certs, r.ltv.embedded_ocsp_responses, r.ltv.embedded_crls, revocation
        );
        println!(
            "  - Pades LTV:   level={:?}, timestamp={:?}, ltv={:?}, indication={:?}/{:?}",
            r.pades_ltv.achieved_pades_level,
            r.pades_ltv.signature_timestamp_status,
            r.pades_ltv.ltv_status,
            r.pades_ltv.validation_indication,
            r.pades_ltv.validation_subindication
        );
        println!(
            "      DSS/VRI replay: matched={}, replayable={}, status={:?}",
            r.pades_ltv.dss.vri_matched,
            r.pades_ltv.dss.evidence_replayable_offline,
            r.pades_ltv.dss.status
        );
        if let Some(c) = &r.certificate {
            println!("  - Certificate:");
            println!("      Subject:  {}", c.subject);
            println!("      Issuer:   {}", c.issuer);
            println!("      Serial:   {}", c.serial_hex);
            println!("      Validity: {} .. {}", c.not_before, c.not_after);
        }
        println!("  - Note: {}", r.note);
        println!();
    }
    finish_signature_cli_validation(&reports, mode)
}

fn run_evidence_export(args: VerifySigArgs) -> Result<(), Box<dyn Error>> {
    if args.evidence_out.is_none() {
        return Err(usage_error(
            "evidence-export requires --evidence-out <bundle.json>",
        ));
    }
    run_verify_sig(args, SignatureCliMode::SignatureVerify)
}

fn run_evidence_fetch(args: VerifySigArgs) -> Result<(), Box<dyn Error>> {
    if args.evidence_out.is_none() {
        return Err(usage_error(
            "evidence-fetch requires --evidence-out <bundle.json>",
        ));
    }
    if !args.online {
        return Err(usage_error(
            "evidence-fetch requires explicit --online bounded retrieval opt-in",
        ));
    }
    run_verify_sig(args, SignatureCliMode::SignatureVerify)
}

fn run_evidence_replay(args: VerifySigArgs) -> Result<(), Box<dyn Error>> {
    if args.evidence_in.is_none() {
        return Err(usage_error(
            "evidence-verify/evidence-replay requires --evidence-in <bundle.json>",
        ));
    }
    if args.online {
        return Err(usage_error(
            "evidence-verify/evidence-replay is offline-only; omit --online",
        ));
    }
    run_verify_sig(args, SignatureCliMode::SignatureVerify)
}

fn run_timestamp_verify(args: TimestampVerifyArgs) -> Result<(), Box<dyn Error>> {
    let token = std::fs::read(&args.token)?;
    let signature_value = std::fs::read(&args.signature_value)?;
    let options = verify_options_from_timestamp_args(&args)?;
    let report = wellfriendpdf_engine::verify_signature_timestamp_token_der(
        &token,
        &signature_value,
        &options,
    )?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("Timestamp token");
        println!("  - Status:       {:?}", report.status);
        println!("  - Type:         {:?}", report.token_type);
        println!("  - Location:     {}", report.location);
        println!("  - SHA-256:      {}", report.raw_token_sha256);
        if let Some(policy) = &report.policy_oid {
            println!("  - Policy:       {policy}");
        }
        if let Some(serial) = &report.serial_hex {
            println!("  - Serial:       {serial}");
        }
        if let Some(gen_time) = &report.gen_time {
            println!("  - genTime:      {gen_time}");
        }
        if let Some(hash) = &report.hash_algorithm {
            println!("  - Imprint hash: {hash}");
        }
        println!("  - Imprint:      {:?}", report.message_imprint_status);
        println!("  - CMS math:     {:?}", report.cms_signature_status);
        println!("  - TSA EKU:      {:?}", report.tsa_eku_status);
        println!("  - TSA path:     {:?}", report.tsa_path_status);
        if !report.errors.is_empty() {
            println!("  - Errors:");
            for error in &report.errors {
                println!("      {error}");
            }
        }
        if !report.warnings.is_empty() {
            println!("  - Warnings:");
            for warning in &report.warnings {
                println!("      {warning}");
            }
        }
    }

    finish_timestamp_cli_validation(&report)
}

fn finish_timestamp_cli_validation(
    report: &wellfriendpdf_engine::TimestampValidationReport,
) -> Result<(), Box<dyn Error>> {
    use wellfriendpdf_engine::SignatureValidationState as State;

    if report.status == State::Valid {
        return Ok(());
    }
    let code = match report.status {
        State::DigestMismatch | State::SignatureMathInvalid | State::Malformed | State::Invalid => {
            CliExitCode::SignatureInvalid
        }
        State::Revoked => CliExitCode::Revoked,
        State::NetworkDisabled | State::NetworkFailure => CliExitCode::Network,
        State::UnsupportedAlgorithm | State::UnsupportedProfile => CliExitCode::Unsupported,
        State::Untrusted
        | State::Expired
        | State::NotYetValid
        | State::PolicyRejected
        | State::PathInvalid
        | State::PathNotFound
        | State::EvidenceMissing
        | State::SignerCertificateMissing
        | State::SignerCertificateAmbiguous => CliExitCode::Untrusted,
        _ => CliExitCode::Indeterminate,
    };
    Err(Box::new(CliError::new(
        code,
        format!(
            "timestamp token validation did not establish validity: {:?}",
            report.status
        ),
    )))
}

fn finish_signature_cli_validation(
    reports: &[wellfriendpdf_engine::SignatureReport],
    mode: SignatureCliMode,
) -> Result<(), Box<dyn Error>> {
    use wellfriendpdf_engine::{
        SignatureStatus, SignatureTrust, SignatureValidationState, SignatureValidity,
    };

    if matches!(
        mode,
        SignatureCliMode::LegacyInspect
            | SignatureCliMode::List
            | SignatureCliMode::SignatureTimestamps
            | SignatureCliMode::DssInspect
            | SignatureCliMode::PadesLevelReport
    ) {
        return Ok(());
    }
    if reports.is_empty() {
        return Err(Box::new(CliError::new(
            CliExitCode::Indeterminate,
            "no digital signatures were available for policy validation",
        )));
    }

    if reports.iter().any(|report| {
        matches!(
            report.validity,
            SignatureValidity::Invalid | SignatureValidity::Error
        )
    }) {
        return Err(Box::new(CliError::new(
            CliExitCode::SignatureInvalid,
            "at least one signature did not pass PDF/CMS cryptographic validation",
        )));
    }
    if reports
        .iter()
        .any(|report| report.validity == SignatureValidity::UnsupportedAlgorithm)
    {
        return Err(Box::new(CliError::new(
            CliExitCode::Unsupported,
            "at least one signature uses an unsupported or policy-forbidden algorithm",
        )));
    }
    if reports
        .iter()
        .any(|report| report.trust == SignatureTrust::Revoked)
    {
        return Err(Box::new(CliError::new(
            CliExitCode::Revoked,
            "authenticated revocation evidence reports at least one certificate revoked",
        )));
    }
    if reports.iter().any(|report| {
        report.signature_validation.network.status == SignatureValidationState::NetworkFailure
            || report.signature_validation.revocation.status
                == SignatureValidationState::NetworkFailure
    }) {
        return Err(Box::new(CliError::new(
            CliExitCode::Network,
            "controlled evidence retrieval failed before policy validation completed",
        )));
    }

    match mode {
        SignatureCliMode::SignatureVerify => {
            if reports
                .iter()
                .all(|report| report.status == SignatureStatus::Trusted)
            {
                Ok(())
            } else if reports.iter().any(|report| {
                matches!(
                    report.trust,
                    SignatureTrust::NotVerified
                        | SignatureTrust::Untrusted
                        | SignatureTrust::Expired
                )
            }) {
                Err(Box::new(CliError::new(
                    CliExitCode::Untrusted,
                    "signature math passed but explicit trust-anchor validation did not establish trusted validity",
                )))
            } else {
                Err(Box::new(CliError::new(
                    CliExitCode::Indeterminate,
                    "signature validation did not establish current-file trusted validity",
                )))
            }
        }
        SignatureCliMode::PadesVerify => {
            if reports.iter().all(|report| {
                report.status == SignatureStatus::Trusted
                    && report.signature_validation.pades.status == SignatureValidationState::Valid
            }) {
                Ok(())
            } else if reports.iter().any(|report| {
                matches!(
                    report.signature_validation.pades.status,
                    SignatureValidationState::UnsupportedProfile
                        | SignatureValidationState::DeferredToLaterPrompt
                        | SignatureValidationState::UnsupportedAlgorithm
                )
            }) {
                Err(Box::new(CliError::new(
                    CliExitCode::Unsupported,
                    "PAdES baseline requirements were not supported or are deferred to Pades LTV",
                )))
            } else {
                Err(Box::new(CliError::new(
                    CliExitCode::Indeterminate,
                    "PAdES baseline did not establish trusted whole-document conformance",
                )))
            }
        }
        SignatureCliMode::LtvVerify => {
            if reports
                .iter()
                .all(|report| report.pades_ltv.ltv_status == SignatureValidationState::Valid)
            {
                Ok(())
            } else if reports.iter().any(|report| {
                matches!(
                    report.pades_ltv.ltv_status,
                    SignatureValidationState::UnsupportedAlgorithm
                        | SignatureValidationState::UnsupportedProfile
                )
            }) {
                Err(Box::new(CliError::new(
                    CliExitCode::Unsupported,
                    "PAdES LTV validation encountered an unsupported timestamp, DSS, or evidence algorithm",
                )))
            } else {
                Err(Box::new(CliError::new(
                    CliExitCode::Indeterminate,
                    "PAdES LTV validation did not establish validated timestamp plus replayable DSS/VRI evidence",
                )))
            }
        }
        SignatureCliMode::CertificatePathBuild => {
            if reports.iter().all(|report| {
                !report
                    .signature_validation
                    .path
                    .selected_path_subjects
                    .is_empty()
            }) {
                Ok(())
            } else if reports.iter().any(|report| {
                report.signature_validation.path.status == SignatureValidationState::PathNotFound
            }) {
                Err(Box::new(CliError::new(
                    CliExitCode::Indeterminate,
                    "certificate path construction did not find a bounded candidate path",
                )))
            } else {
                Err(Box::new(CliError::new(
                    CliExitCode::Untrusted,
                    "certificate path construction did not produce a usable signer path",
                )))
            }
        }
        SignatureCliMode::CertificatePathVerify => {
            if reports.iter().all(|report| {
                report.signature_validation.path.status == SignatureValidationState::Valid
            }) {
                Ok(())
            } else if reports.iter().any(|report| {
                matches!(
                    report.signature_validation.path.status,
                    SignatureValidationState::Untrusted
                        | SignatureValidationState::PathNotFound
                        | SignatureValidationState::NotChecked
                )
            }) {
                Err(Box::new(CliError::new(
                    CliExitCode::Untrusted,
                    "certificate path validation did not terminate at an explicit trusted anchor",
                )))
            } else {
                Err(Box::new(CliError::new(
                    CliExitCode::Indeterminate,
                    "certificate path validation did not establish a valid RFC 5280 path",
                )))
            }
        }
        SignatureCliMode::OcspCheck => finish_revocation_evidence_cli(reports, "ocsp", "OCSP"),
        SignatureCliMode::CrlCheck => finish_revocation_evidence_cli(reports, "crl", "CRL"),
        SignatureCliMode::RevocationCheck => {
            if reports.iter().all(|report| {
                report.signature_validation.revocation.status == SignatureValidationState::Valid
            }) {
                Ok(())
            } else {
                Err(Box::new(CliError::new(
                    CliExitCode::Indeterminate,
                    "revocation evidence did not establish fresh good status for every required certificate",
                )))
            }
        }
        SignatureCliMode::LegacyInspect
        | SignatureCliMode::List
        | SignatureCliMode::SignatureTimestamps
        | SignatureCliMode::DssInspect
        | SignatureCliMode::PadesLevelReport => Ok(()),
    }
}

fn finish_revocation_evidence_cli(
    reports: &[wellfriendpdf_engine::SignatureReport],
    evidence_prefix: &str,
    label: &str,
) -> Result<(), Box<dyn Error>> {
    if reports.iter().any(|report| {
        report.signature_validation.revocation.status
            == wellfriendpdf_engine::SignatureValidationState::Revoked
    }) {
        return Err(Box::new(CliError::new(
            CliExitCode::Revoked,
            format!("{label} revocation evidence reports a revoked certificate"),
        )));
    }
    if reports.iter().all(|report| {
        report.signature_validation.revocation.status
            == wellfriendpdf_engine::SignatureValidationState::Valid
            && report
                .signature_validation
                .revocation
                .certificate_decisions
                .iter()
                .any(|decision| {
                    decision
                        .evidence_type
                        .as_deref()
                        .is_some_and(|evidence_type| evidence_type.starts_with(evidence_prefix))
                })
    }) {
        Ok(())
    } else {
        Err(Box::new(CliError::new(
            CliExitCode::Indeterminate,
            format!("{label} evidence did not establish fresh good status for every required certificate"),
        )))
    }
}

fn verify_options_from_args(
    args: &VerifySigArgs,
) -> Result<wellfriendpdf_engine::VerifyOptions, Box<dyn Error>> {
    let mut options = wellfriendpdf_engine::VerifyOptions::default();
    let mut trust_store = wellfriendpdf_engine::TrustStore::new();
    for path in &args.trust_anchors {
        let bytes = std::fs::read(path)?;
        if certificate_input_is_pem(&bytes) {
            trust_store.add_pem(&bytes, path.display().to_string(), None)?;
        } else {
            trust_store.add_der(&bytes, path.display().to_string(), None)?;
        }
    }
    options = options.with_trust_store(&trust_store);

    let mut intermediate_store = wellfriendpdf_engine::IntermediateStore::new();
    for path in &args.intermediates {
        let bytes = std::fs::read(path)?;
        if certificate_input_is_pem(&bytes) {
            intermediate_store.add_pem(&bytes)?;
        } else {
            intermediate_store.add_der(&bytes)?;
        }
    }
    options = options.with_intermediate_store(&intermediate_store);
    for fingerprint in &args.distrust_certificate_sha256 {
        options = options.with_distrusted_certificate_sha256(fingerprint)?;
    }
    for path in &args.ocsp_responses {
        options = options.with_ocsp_response_der(std::fs::read(path)?);
    }
    for path in &args.crls {
        options = options.with_crl_der(std::fs::read(path)?);
    }
    if args.online {
        let mut policy = wellfriendpdf_engine::RetrievalPolicy::online();
        if let Some(timeout_ms) = args.network_timeout_ms {
            if timeout_ms == 0 {
                return Err(usage_error(
                    "--network-timeout-ms must be greater than zero",
                ));
            }
            policy.budget.total_timeout_ms = timeout_ms;
            policy.budget.connect_timeout_ms = policy.budget.connect_timeout_ms.min(timeout_ms);
            policy.budget.max_total_time_ms = timeout_ms;
        }
        if let Some(max_bytes) = args.network_max_response_bytes {
            if max_bytes == 0 || max_bytes > policy.budget.max_response_bytes {
                return Err(usage_error(format!(
                    "--network-max-response-bytes must be between 1 and {}",
                    policy.budget.max_response_bytes
                )));
            }
            policy.budget.max_response_bytes = max_bytes;
        }
        if let Some(cache_dir) = &args.cache_dir {
            policy.cache_directory = Some(cache_dir.to_string_lossy().to_string());
        }
        policy.allowed_hosts = args.network_allow_hosts.clone();
        if args.ocsp_require_nonce {
            policy.ocsp_nonce_policy = wellfriendpdf_engine::OcspNoncePolicy::Required;
        }
        options = options.with_retrieval_policy(policy)?;
    } else if !args.network_allow_hosts.is_empty()
        || args.network_timeout_ms.is_some()
        || args.network_max_response_bytes.is_some()
        || args.cache_dir.is_some()
        || args.ocsp_require_nonce
    {
        return Err(usage_error(
            "network policy flags require explicit --online opt-in",
        ));
    }
    if let Some(path) = &args.evidence_in {
        let bytes = std::fs::read(path)?;
        let bundle: wellfriendpdf_engine::EvidenceBundle = serde_json::from_slice(&bytes)
            .map_err(|error| usage_error(format!("invalid --evidence-in bundle: {error}")))?;
        options = options.with_evidence_bundle(bundle)?;
    }
    if let Some(path) = &args.algorithm_policy {
        let bytes = std::fs::read(path)?;
        let policy: wellfriendpdf_engine::SignatureAlgorithmPolicy = serde_json::from_slice(&bytes)
            .map_err(|error| usage_error(format!("invalid --algorithm-policy JSON: {error}")))?;
        options = options.with_algorithm_policy(policy)?;
    }
    if let Some(unix) = args.validation_time_unix {
        options = options.with_validation_time_unix(unix);
    }
    options.revocation_mode = parse_signature_revocation_mode(&args.revocation)?;
    Ok(options)
}

fn verify_options_from_timestamp_args(
    args: &TimestampVerifyArgs,
) -> Result<wellfriendpdf_engine::VerifyOptions, Box<dyn Error>> {
    let mut options = wellfriendpdf_engine::VerifyOptions::default();
    let mut trust_store = wellfriendpdf_engine::TrustStore::new();
    for path in &args.trust_anchors {
        let bytes = std::fs::read(path)?;
        if certificate_input_is_pem(&bytes) {
            trust_store.add_pem(&bytes, path.display().to_string(), None)?;
        } else {
            trust_store.add_der(&bytes, path.display().to_string(), None)?;
        }
    }
    options = options.with_trust_store(&trust_store);

    let mut intermediate_store = wellfriendpdf_engine::IntermediateStore::new();
    for path in &args.intermediates {
        let bytes = std::fs::read(path)?;
        if certificate_input_is_pem(&bytes) {
            intermediate_store.add_pem(&bytes)?;
        } else {
            intermediate_store.add_der(&bytes)?;
        }
    }
    options = options.with_intermediate_store(&intermediate_store);
    for fingerprint in &args.distrust_certificate_sha256 {
        options = options.with_distrusted_certificate_sha256(fingerprint)?;
    }
    for path in &args.ocsp_responses {
        options = options.with_ocsp_response_der(std::fs::read(path)?);
    }
    for path in &args.crls {
        options = options.with_crl_der(std::fs::read(path)?);
    }
    if args.online {
        let mut policy = wellfriendpdf_engine::RetrievalPolicy::online();
        if let Some(timeout_ms) = args.network_timeout_ms {
            if timeout_ms == 0 {
                return Err(usage_error(
                    "--network-timeout-ms must be greater than zero",
                ));
            }
            policy.budget.total_timeout_ms = timeout_ms;
            policy.budget.connect_timeout_ms = policy.budget.connect_timeout_ms.min(timeout_ms);
            policy.budget.max_total_time_ms = timeout_ms;
        }
        if let Some(max_bytes) = args.network_max_response_bytes {
            if max_bytes == 0 || max_bytes > policy.budget.max_response_bytes {
                return Err(usage_error(format!(
                    "--network-max-response-bytes must be between 1 and {}",
                    policy.budget.max_response_bytes
                )));
            }
            policy.budget.max_response_bytes = max_bytes;
        }
        if let Some(cache_dir) = &args.cache_dir {
            policy.cache_directory = Some(cache_dir.to_string_lossy().to_string());
        }
        policy.allowed_hosts = args.network_allow_hosts.clone();
        if args.ocsp_require_nonce {
            policy.ocsp_nonce_policy = wellfriendpdf_engine::OcspNoncePolicy::Required;
        }
        options = options.with_retrieval_policy(policy)?;
    } else if !args.network_allow_hosts.is_empty()
        || args.network_timeout_ms.is_some()
        || args.network_max_response_bytes.is_some()
        || args.cache_dir.is_some()
        || args.ocsp_require_nonce
    {
        return Err(usage_error(
            "network policy flags require explicit --online opt-in",
        ));
    }
    if let Some(path) = &args.evidence_in {
        let bytes = std::fs::read(path)?;
        let bundle: wellfriendpdf_engine::EvidenceBundle = serde_json::from_slice(&bytes)
            .map_err(|error| usage_error(format!("invalid --evidence-in bundle: {error}")))?;
        options = options.with_evidence_bundle(bundle)?;
    }
    if let Some(path) = &args.algorithm_policy {
        let bytes = std::fs::read(path)?;
        let policy: wellfriendpdf_engine::SignatureAlgorithmPolicy = serde_json::from_slice(&bytes)
            .map_err(|error| usage_error(format!("invalid --algorithm-policy JSON: {error}")))?;
        options = options.with_algorithm_policy(policy)?;
    }
    if let Some(unix) = args.validation_time_unix {
        options = options.with_validation_time_unix(unix);
    }
    options.revocation_mode = parse_signature_revocation_mode(&args.revocation)?;
    Ok(options)
}

fn certificate_input_is_pem(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes)
        .map(|text| text.trim_start().starts_with("-----BEGIN"))
        .unwrap_or(false)
}

fn write_evidence_bundle(
    path: &Path,
    bundle: &wellfriendpdf_engine::EvidenceBundle,
    options: &wellfriendpdf_engine::VerifyOptions,
) -> Result<(), Box<dyn Error>> {
    let budget = &options.retrieval_policy.budget;
    let store = wellfriendpdf_engine::EvidenceStore::import_bundle(
        bundle,
        budget.max_cache_entries,
        budget.max_cache_bytes,
    )
    .map_err(|error| {
        usage_error(format!(
            "refusing to export invalid evidence bundle: {error}"
        ))
    })?;
    store
        .write_bundle_atomically(
            path,
            bundle.source_document_sha256.clone(),
            bundle.signature_identifier.clone(),
            bundle.validation_time_unix,
            bundle.policy_sha256.clone(),
        )
        .map_err(|error| usage_error(format!("evidence export: {error}")))?;
    Ok(())
}

fn parse_signature_revocation_mode(
    value: &str,
) -> Result<wellfriendpdf_engine::SignatureRevocationMode, Box<dyn Error>> {
    match value {
        "not-checked" | "not_checked" => Ok(wellfriendpdf_engine::SignatureRevocationMode::NotChecked),
        "offline-strict" | "offline_strict" => {
            Ok(wellfriendpdf_engine::SignatureRevocationMode::OfflineStrict)
        }
        "offline-best-effort" | "offline_best_effort" => {
            Ok(wellfriendpdf_engine::SignatureRevocationMode::OfflineBestEffort)
        }
        "online-strict" | "online_strict" | "online-hard-fail" | "online_hard_fail" => {
            Ok(wellfriendpdf_engine::SignatureRevocationMode::OnlineStrict)
        }
        "online-best-effort"
        | "online_best_effort"
        | "online-best-evidence"
        | "online_best_evidence" => Ok(wellfriendpdf_engine::SignatureRevocationMode::OnlineBestEffort),
        other => Err(format!(
            "unknown revocation mode '{other}'; use not-checked, offline-strict, offline-best-effort, online-strict, or online-best-effort"
        )
        .into()),
    }
}

fn run_security_report(args: SecurityReportArgs) -> Result<(), Box<dyn Error>> {
    let engine = open_engine(&args.pdf, &args.password)?;
    let report = wellfriendpdf_engine::security_report(&engine)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("Security report");
        println!("  encrypted: {}", report.encrypted);
        println!("  signatures: {}", report.signatures.len());
        println!("  risky content: {}", report.risky_content.risky_total());
        println!(
            "  public-key security handler: {}",
            report.public_key_security_handler_detected
        );
        println!("  AES-GCM detected: {}", report.aes_gcm_detected);
        for finding in &report.findings {
            println!(
                "  - {:?} {}: {}",
                finding.severity, finding.code, finding.message
            );
        }
    }
    Ok(())
}

fn run_sanitize(args: SanitizeArgs) -> Result<(), Box<dyn Error>> {
    let engine = open_engine(&args.pdf, &args.password)?;
    let options = parse_sanitizer_options(&args.policy)?;
    let (bytes, report) = wellfriendpdf_engine::sanitize_pdf(&engine, &options)?;
    std::fs::write(&args.output, &bytes)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "Sanitized {} -> {} (risky: {} -> {}, strict_passed={})",
            args.pdf.display(),
            args.output.display(),
            report.input_risky_total,
            report.output_risky_total,
            report.strict_passed
        );
    }
    if args.strict && !report.strict_passed {
        return Err("sanitizer strict verification failed: risky content remains".into());
    }
    Ok(())
}

fn run_validate(args: ValidateArgs) -> Result<(), Box<dyn Error>> {
    let engine = open_engine(&args.pdf, &args.password)?;
    let profile = wellfriendpdf_engine::StandardsProfile::parse(&args.profile)
        .ok_or_else(|| usage_error(format!("unknown validation profile '{}'", args.profile)))?;
    let report = wellfriendpdf_engine::validate_standards_profile(&engine, profile)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "Validation {:?}: {} rule(s), passed={}",
            report.profile,
            report.rules.len(),
            report.passed
        );
        for rule in &report.rules {
            println!(
                "  - {:?} {:?} {}: {}",
                rule.status, rule.severity, rule.rule_id, rule.message
            );
        }
    }
    let has_fail = report
        .rules
        .iter()
        .any(|rule| matches!(rule.status, wellfriendpdf_engine::ValidationStatus::Fail));
    let has_warn = report
        .rules
        .iter()
        .any(|rule| matches!(rule.status, wellfriendpdf_engine::ValidationStatus::Warn));
    let fail_on = parse_validate_fail_on(&args.fail_on, args.fail_on_warning)?;
    if (fail_on == "error" && has_fail) || (fail_on == "warning" && (has_fail || has_warn)) {
        return Err(Box::new(CliError::new(
            CliExitCode::Input,
            "validation profile reported failing rules",
        )));
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum StandardsKind {
    PdfA,
    PdfUa,
    PdfX,
    All,
}

fn run_standards_validate(
    args: StandardsValidateArgs,
    kind: StandardsKind,
) -> Result<(), Box<dyn Error>> {
    let engine = open_engine(&args.pdf, &args.password)?;
    let options = match &args.target {
        Some(target) => wellfriendpdf_engine::StandardsValidationOptions::with_target(target),
        None => wellfriendpdf_engine::StandardsValidationOptions::default(),
    };
    let (json, conformant, has_fail, has_warn, summary) = match kind {
        StandardsKind::All => {
            let report = wellfriendpdf_engine::validate_all_standards(&engine, &options)?;
            let has_fail = report.reports.iter().any(|r| {
                matches!(
                    r.conformance,
                    wellfriendpdf_engine::ConformanceStatus::NonConformant
                )
            });
            let has_warn = report.conflicts.iter().any(|c| {
                matches!(
                    c.severity,
                    wellfriendpdf_engine::standards_engine::ValidationSeverity::Warning
                )
            });
            let summary = format!(
                "standards: {} profile report(s), {} conflict(s), overall_pass={}",
                report.reports.len(),
                report.conflicts.len(),
                report.overall_pass
            );
            (
                serde_json::to_string_pretty(&report)?,
                report.overall_pass,
                has_fail,
                has_warn,
                summary,
            )
        }
        other => {
            let family = match other {
                StandardsKind::PdfA => wellfriendpdf_engine::StandardsFamily::PdfA,
                StandardsKind::PdfUa => wellfriendpdf_engine::StandardsFamily::PdfUa,
                StandardsKind::PdfX => wellfriendpdf_engine::StandardsFamily::PdfX,
                StandardsKind::All => unreachable!(),
            };
            let report =
                wellfriendpdf_engine::validate_standards_family(&engine, family, &options)?;
            let has_fail = report
                .rules
                .iter()
                .any(|r| matches!(r.status, wellfriendpdf_engine::RuleStatus::Fail));
            let has_warn = report
                .rules
                .iter()
                .any(|r| matches!(r.status, wellfriendpdf_engine::RuleStatus::Warning));
            let summary = format!(
                "{}: {:?}, {} rule(s), {} pass / {} fail / {} unsupported / {} deferred",
                family.as_str(),
                report.conformance,
                report.counts.total,
                report.counts.pass,
                report.counts.fail,
                report.counts.unsupported_reported_exact,
                report.counts.deferred_crypto_standards_fuzz_corpus_parity
            );
            (
                serde_json::to_string_pretty(&report)?,
                report.is_conformant(),
                has_fail,
                has_warn,
                summary,
            )
        }
    };

    if let Some(path) = &args.output_json {
        std::fs::write(path, json.as_bytes())?;
    }
    if args.json {
        println!("{json}");
    } else {
        println!("{summary}");
        println!("  conformant: {conformant}");
    }

    let fail_on = parse_validate_fail_on(&args.fail_on, args.fail_on_warning)?;
    if (fail_on == "error" && has_fail) || (fail_on == "warning" && (has_fail || has_warn)) {
        return Err(Box::new(CliError::new(
            CliExitCode::Input,
            "standards validation reported failing rules",
        )));
    }
    Ok(())
}

fn run_signature_sign(args: SignatureSignArgs, plan_only: bool) -> Result<(), Box<dyn Error>> {
    if !plan_only && args.output.exists() && !args.force {
        return Err(usage_error(format!(
            "output {} already exists; pass --force to overwrite",
            args.output.display()
        )));
    }
    let engine = open_engine(&args.pdf, &args.password)?;
    // Read key/cert material. These bytes are never logged.
    let key_pem = std::fs::read_to_string(&args.key)?;
    let cert_pem = std::fs::read_to_string(&args.cert)?;
    let chain_pem: Vec<String> = args
        .chain
        .iter()
        .map(std::fs::read_to_string)
        .collect::<std::result::Result<_, _>>()?;
    let chain_refs: Vec<&str> = chain_pem.iter().map(String::as_str).collect();
    let signer = wellfriendpdf_engine::PdfSigner::from_pem(&key_pem, &cert_pem, &chain_refs)?;

    let mut signature = wellfriendpdf_engine::SignatureOptions {
        contents_reserved_bytes: args.placeholder_size,
        ..Default::default()
    };
    if let Some(field) = args.field_name.clone() {
        signature.field_name = field;
    }
    signature.reason = args.reason.clone();
    let intent = match args.certify {
        Some(p) => wellfriendpdf_engine::SigningIntent::Certification {
            docmdp_permissions: p,
        },
        None => wellfriendpdf_engine::SigningIntent::Approval,
    };
    let options = wellfriendpdf_engine::IncrementalSigningOptions {
        signature,
        intent,
        retry_larger_placeholder: true,
        max_placeholder_bytes: 256 * 1024,
    };

    if plan_only {
        let plan =
            wellfriendpdf_engine::plan_signature_placeholder(engine.document(), &signer, &options)?;
        if args.json {
            println!("{}", serde_json::to_string_pretty(&plan)?);
        } else {
            println!(
                "placeholder plan: required={} reserved={} fits={} byte_range={:?}",
                plan.required_bytes, plan.reserved_bytes, plan.fits, plan.byte_range
            );
        }
        return Ok(());
    }

    let result = wellfriendpdf_engine::sign_incremental(
        engine.document(),
        wellfriendpdf_engine::IncrementalSigner::Local(&signer),
        &options,
    )?;
    if !result.post_sign.signature_valid {
        return Err(Box::new(CliError::new(
            CliExitCode::Input,
            "post-sign validation failed; signed output not written",
        )));
    }
    std::fs::write(&args.output, &result.signed_pdf)?;
    if args.json {
        // signed_pdf is skip_serializing; the report carries no key material.
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!(
            "Signed {} -> {} (certification={}, reserved={}, cms={} bytes, retried={}, post_sign_valid={}, coverage_whole_file={})",
            args.pdf.display(),
            args.output.display(),
            result.certification,
            result.reserved_bytes,
            result.cms_len,
            result.retried,
            result.post_sign.signature_valid,
            result.post_sign.coverage_whole_file
        );
    }
    Ok(())
}

fn run_canonicalize(args: CanonicalizeArgs) -> Result<(), Box<dyn Error>> {
    let engine = open_engine(&args.pdf, &args.password)?;
    let options = wellfriendpdf_engine::CanonicalizeOptions {
        fixed_source_date_epoch: args.source_date_epoch,
        ..Default::default()
    };
    let (bytes, report) = wellfriendpdf_engine::canonicalize_pdf(&engine, &options)?;
    std::fs::write(&args.output, &bytes)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "Canonicalized {} -> {} (sha256={})",
            args.pdf.display(),
            args.output.display(),
            report.output_sha256
        );
    }
    Ok(())
}

fn parse_sanitizer_options(
    policy: &str,
) -> Result<wellfriendpdf_engine::SanitizerOptions, Box<dyn Error>> {
    match policy.to_ascii_lowercase().as_str() {
        "strict" => Ok(wellfriendpdf_engine::SanitizerOptions::strict()),
        "balanced" => Ok(wellfriendpdf_engine::SanitizerOptions::balanced()),
        "preserve-visual" | "preserve_visual" => {
            Ok(wellfriendpdf_engine::SanitizerOptions::preserve_visual())
        }
        other => Err(usage_error(format!("unknown sanitizer policy '{other}'"))),
    }
}

fn parse_validate_fail_on(
    value: &str,
    fail_on_warning: bool,
) -> Result<&'static str, Box<dyn Error>> {
    if fail_on_warning {
        return Ok("warning");
    }
    match value.to_ascii_lowercase().as_str() {
        "never" | "none" => Ok("never"),
        "error" | "errors" => Ok("error"),
        "warning" | "warnings" | "warn" => Ok("warning"),
        other => Err(usage_error(format!(
            "unknown --fail-on value '{other}'; use never, error, or warning"
        ))),
    }
}

fn run_merge(args: MergeArgs) -> Result<(), Box<dyn Error>> {
    use wellfriendpdf_engine::{build_merged, ContentEngine};

    // Split positional passwords (comma-separated) and match by input index.
    let passwords: Vec<String> = args
        .passwords
        .as_deref()
        .map(|s| s.split(',').map(|p| p.trim().to_string()).collect())
        .unwrap_or_default();

    // Open every input first; keep the engines alive for the whole merge so the
    // builder can borrow each document.
    let mut engines = Vec::with_capacity(args.inputs.len());
    for (idx, path) in args.inputs.iter().enumerate() {
        let engine = match passwords.get(idx) {
            Some(pw) if !pw.is_empty() => {
                ContentEngine::open_path_with_password(path, pw.as_bytes())?
            }
            _ => ContentEngine::open_path(path)?,
        };
        engines.push(engine);
    }

    // Take all pages of each input, in order.
    let mut inputs = Vec::with_capacity(engines.len());
    let mut total_pages = 0usize;
    for engine in &engines {
        let count = engine.page_count()?;
        total_pages += count;
        let all: Vec<usize> = (1..=count).collect();
        inputs.push((engine.document(), all));
    }

    let bytes = build_merged(&inputs)?;
    std::fs::write(&args.output, &bytes)?;
    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "op": "merge",
                "output": args.output.display().to_string(),
                "inputs": engines.len(),
                "pages": total_pages,
                "bytes": bytes.len(),
            })
        );
    } else {
        eprintln!(
            "Merged {} file(s), {} page(s) -> {}",
            engines.len(),
            total_pages,
            args.output.display()
        );
    }
    Ok(())
}

fn run_split(args: SplitArgs) -> Result<(), Box<dyn Error>> {
    use wellfriendpdf_engine::ContentEngine;

    let engine = match &args.password {
        Some(pw) => ContentEngine::open_path_with_password(&args.pdf, pw.as_bytes())?,
        None => ContentEngine::open_path(&args.pdf)?,
    };
    let total = engine.page_count()?;
    if total == 0 {
        return Err("document has no pages".into());
    }

    let first = args.first.unwrap_or(1);
    let last = args.last.unwrap_or(total);
    if first == 0 || first > total {
        return Err(format!("--first {first} is out of range (1..={total})").into());
    }
    if last < first || last > total {
        return Err(format!("--last {last} is out of range ({first}..={total})").into());
    }

    let mut written = 0usize;
    for page in first..=last {
        let bytes = engine.extract_single_page(page)?;
        let path = expand_split_pattern(&args.output, page);
        std::fs::write(&path, &bytes)?;
        written += 1;
    }
    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "op": "split",
                "pattern": args.output,
                "first": first,
                "last": last,
                "files": written,
            })
        );
    } else {
        eprintln!(
            "Split {} page(s) [{}..={}] using pattern '{}'",
            written, first, last, args.output
        );
    }
    Ok(())
}

fn run_extract_pages(args: ExtractPagesArgs) -> Result<(), Box<dyn Error>> {
    use wellfriendpdf_engine::ContentEngine;

    let engine = match &args.password {
        Some(pw) => ContentEngine::open_path_with_password(&args.pdf, pw.as_bytes())?,
        None => ContentEngine::open_path(&args.pdf)?,
    };
    let total = engine.page_count()?;
    let pages = parse_page_selection_ordered(&args.pages, total)?;
    if pages.is_empty() {
        return Err(format!("selection '{}' matched no pages in 1..={total}", args.pages).into());
    }

    let bytes = engine.extract_pages(&pages)?;
    std::fs::write(&args.output, &bytes)?;
    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "op": "extract-pages",
                "output": args.output.display().to_string(),
                "pages": pages,
                "bytes": bytes.len(),
            })
        );
    } else {
        eprintln!(
            "Extracted {} page(s) -> {}",
            pages.len(),
            args.output.display()
        );
    }
    Ok(())
}

// --- Structural-write ops (Bucket 2): encrypt / rotate / optimize / repair ---

fn run_encrypt(args: EncryptArgs) -> Result<(), Box<dyn Error>> {
    use wellfriendpdf_engine::crypto::{secret_bytes, EncryptAlgorithm, EncryptParams};

    let algo = EncryptAlgorithm::parse(&args.algo).ok_or_else(|| {
        format!(
            "unknown --algo '{}'; use aes256, aesgcm, aes128, or rc4",
            args.algo
        )
    })?;
    let engine = open_engine(&args.pdf, &args.password)?;
    let owner = if args.owner_pw.is_empty() {
        args.user_pw.clone()
    } else {
        args.owner_pw.clone()
    };
    let params = EncryptParams {
        user_password: secret_bytes(args.user_pw.into_bytes()),
        owner_password: secret_bytes(owner.into_bytes()),
        permissions: args.permissions,
        algorithm: algo,
        encrypt_metadata: true,
    };
    if matches!(algo, EncryptAlgorithm::Rc4_128 | EncryptAlgorithm::Aes128) {
        eprintln!(
            "Warning: {} is a legacy algorithm. Wellfriend reads its own output, but \
             cross-reader interop is only verified for AES-256 (the default). \
             Prefer --algo aes256 unless a consumer requires legacy encryption.",
            args.algo
        );
    }
    let bytes = wellfriendpdf_engine::encrypt(&engine, &params)?;
    std::fs::write(&args.output, &bytes)?;
    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "op": "encrypt",
                "output": args.output.display().to_string(),
                "algorithm": args.algo,
                "bytes": bytes.len(),
            })
        );
    } else {
        eprintln!(
            "Encrypted with {} -> {} ({} bytes)",
            args.algo,
            args.output.display(),
            bytes.len()
        );
    }
    Ok(())
}

fn run_decrypt(args: DecryptArgs) -> Result<(), Box<dyn Error>> {
    let engine = open_engine(&args.pdf, &args.password)?;
    let bytes = wellfriendpdf_engine::decrypt_pdf(&engine)?;
    std::fs::write(&args.output, &bytes)?;
    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "op": "decrypt",
                "output": args.output.display().to_string(),
                "bytes": bytes.len(),
            })
        );
    } else {
        eprintln!(
            "Wrote unencrypted copy -> {} ({} bytes)",
            args.output.display(),
            bytes.len()
        );
    }
    Ok(())
}

fn run_watermark(args: WatermarkArgs) -> Result<(), Box<dyn Error>> {
    let has_text = args.text.is_some();
    let has_image = args.image.is_some();
    if has_text == has_image {
        return Err("pass exactly one of --text or --image".into());
    }
    let input = read_edit_input(&args.pdf, &args.password)?;
    let engine = wellfriendpdf_engine::ContentEngine::open_bytes(input.clone())?;
    let total = engine.page_count()?;
    let pages = parse_page_range_cli(&args.pages, total)?;
    let position = parse_stamp_position(&args.position)?;
    let bytes = if let Some(text) = args.text {
        wellfriendpdf_engine::watermark_text_pdf(
            input,
            &text,
            wellfriendpdf_engine::TextWatermarkOptions {
                pages: pages.clone(),
                position,
                opacity: args.opacity,
                rotation_degrees: args.rotation,
                font_size: args.font_size,
                color: parse_rgb_color(&args.color)?,
            },
        )?
    } else {
        let image_path = args.image.expect("checked above");
        let image = std::fs::read(&image_path)?;
        wellfriendpdf_engine::watermark_image_pdf(
            input,
            &image,
            image_path.extension().and_then(|s| s.to_str()),
            wellfriendpdf_engine::ImageWatermarkOptions {
                pages: pages.clone(),
                position,
                opacity: args.opacity,
                scale: args.scale,
            },
        )?
    };
    std::fs::write(&args.output, &bytes)?;
    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "op": "watermark",
                "output": args.output.display().to_string(),
                "pages": pages,
                "bytes": bytes.len(),
            })
        );
    } else {
        eprintln!(
            "Watermarked {} page(s) -> {}",
            pages.len(),
            args.output.display()
        );
    }
    Ok(())
}

fn run_page_numbers(args: PageNumbersArgs) -> Result<(), Box<dyn Error>> {
    let input = read_edit_input(&args.pdf, &args.password)?;
    let engine = wellfriendpdf_engine::ContentEngine::open_bytes(input.clone())?;
    let total = engine.page_count()?;
    let pages = parse_page_range_cli(&args.pages, total)?;
    let bytes = wellfriendpdf_engine::add_page_numbers_pdf(
        input,
        wellfriendpdf_engine::PageNumberOptions {
            pages: pages.clone(),
            position: parse_stamp_position(&args.position)?,
            format: args.format,
            start: args.start,
            font_size: args.font_size,
            color: parse_rgb_color(&args.color)?,
        },
    )?;
    std::fs::write(&args.output, &bytes)?;
    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "op": "add-page-numbers",
                "output": args.output.display().to_string(),
                "pages": pages,
                "bytes": bytes.len(),
            })
        );
    } else {
        eprintln!(
            "Added page numbers to {} page(s) -> {}",
            pages.len(),
            args.output.display()
        );
    }
    Ok(())
}

fn run_organize(args: OrganizeArgs) -> Result<(), Box<dyn Error>> {
    let engine = open_engine(&args.pdf, &args.password)?;
    let total = engine.page_count()?;
    let order = parse_page_selection_ordered(&args.order, total)?;
    let bytes = if let Some(insert_path) = args.insert_from {
        let inserted = open_engine(&insert_path, &args.insert_password)?;
        let inserted_total = inserted.page_count()?;
        let insert_pages = parse_page_selection_ordered(&args.insert_pages, inserted_total)?;
        wellfriendpdf_engine::organize_pdf_with_insert(
            &engine,
            &order,
            Some((&inserted, insert_pages, args.insert_at)),
        )?
    } else {
        wellfriendpdf_engine::organize_pdf(&engine, &order)?
    };
    std::fs::write(&args.output, &bytes)?;
    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "op": "organize",
                "output": args.output.display().to_string(),
                "order": order,
                "bytes": bytes.len(),
            })
        );
    } else {
        eprintln!("Organized PDF -> {}", args.output.display());
    }
    Ok(())
}

fn run_rotate(args: RotateArgs) -> Result<(), Box<dyn Error>> {
    use wellfriendpdf_engine::Rotation;

    let engine = open_engine(&args.pdf, &args.password)?;
    let total = engine.page_count()?;
    let pages = parse_page_range_cli(&args.pages, total)?;
    let rotation = if args.relative {
        Rotation::Relative(args.angle)
    } else {
        Rotation::Absolute(args.angle)
    };
    let bytes = wellfriendpdf_engine::rotate_pages(&engine, &pages, rotation)?;
    std::fs::write(&args.output, &bytes)?;
    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "op": "rotate",
                "output": args.output.display().to_string(),
                "angle": args.angle,
                "relative": args.relative,
                "pages": pages.len(),
            })
        );
    } else {
        eprintln!(
            "Rotated {} page(s) by {}{} -> {}",
            pages.len(),
            if args.relative { "+" } else { "" },
            args.angle,
            args.output.display()
        );
    }
    Ok(())
}

fn run_pages_crop(args: PagesCropArgs) -> Result<(), Box<dyn Error>> {
    let engine = open_engine(&args.pdf, &args.password)?;
    let total = engine.page_count()?;
    let pages = parse_page_range_cli(&args.pages, total)?;
    let rect = parse_plain_rect_cli(&args.rect)?;
    let bytes = wellfriendpdf_engine::crop_pdf(&engine, &pages, rect)?;
    std::fs::write(&args.output, &bytes)?;
    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "op": "pages-crop",
                "output": args.output.display().to_string(),
                "bytes": bytes.len(),
                "pages": pages,
                "crop_box": [rect.x, rect.y, rect.width, rect.height],
                "preservation": "source_graph_preserved",
            })
        );
    } else {
        eprintln!(
            "Cropped {} page(s) -> {}",
            pages.len(),
            args.output.display()
        );
    }
    Ok(())
}

fn run_pages_scale(args: PagesScaleArgs) -> Result<(), Box<dyn Error>> {
    let engine = open_engine(&args.pdf, &args.password)?;
    let total = engine.page_count()?;
    let pages = parse_page_range_cli(&args.pages, total)?;
    let options = wellfriendpdf_engine::ScalePagesOptions {
        pages: Some(pages.clone()),
        scale: args.scale,
        dpi: args.dpi,
    };
    let bytes = wellfriendpdf_engine::scale_pdf_pages(&engine, options)?;
    std::fs::write(&args.output, &bytes)?;
    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "op": "pages-scale",
                "output": args.output.display().to_string(),
                "bytes": bytes.len(),
                "pages": pages,
                "scale": args.scale,
                "dpi": args.dpi,
                "preservation": "visual_raster_copy_interactive_structures_not_preserved",
            })
        );
    } else {
        eprintln!(
            "Scaled {} page(s) -> {}",
            pages.len(),
            args.output.display()
        );
    }
    Ok(())
}

fn run_pages_nup(args: PagesNupArgs) -> Result<(), Box<dyn Error>> {
    let engine = open_engine(&args.pdf, &args.password)?;
    let total = engine.page_count()?;
    let pages = parse_page_range_cli(&args.pages, total)?;
    let options = wellfriendpdf_engine::NUpOptions {
        columns: args.columns,
        rows: args.rows,
        dpi: args.dpi,
    };
    let bytes = wellfriendpdf_engine::n_up_pdf(&engine, &pages, options)?;
    std::fs::write(&args.output, &bytes)?;
    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "op": "pages-nup",
                "output": args.output.display().to_string(),
                "bytes": bytes.len(),
                "pages": pages,
                "columns": args.columns,
                "rows": args.rows,
                "dpi": args.dpi,
                "preservation": "visual_imposition_interactive_structures_not_preserved",
            })
        );
    } else {
        eprintln!("Created n-up PDF -> {}", args.output.display());
    }
    Ok(())
}

fn run_optimize(args: OptimizeArgs) -> Result<(), Box<dyn Error>> {
    let input_size = std::fs::metadata(&args.pdf)
        .map(|m| m.len() as usize)
        .unwrap_or(0);
    let engine = open_engine(&args.pdf, &args.password)?;
    let (bytes, report) = wellfriendpdf_engine::optimize(&engine)?;
    std::fs::write(&args.output, &bytes)?;
    let saved = input_size.saturating_sub(bytes.len());
    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "op": "optimize",
                "output": args.output.display().to_string(),
                "input_bytes": input_size,
                "output_bytes": bytes.len(),
                "saved_bytes": saved,
                "streams_recompressed": report.streams_recompressed,
            })
        );
    } else {
        eprintln!(
            "Optimized {} -> {} bytes ({} saved, {} stream(s) recompressed) -> {}",
            input_size,
            bytes.len(),
            saved,
            report.streams_recompressed,
            args.output.display()
        );
    }
    Ok(())
}

fn run_repair(args: RepairArgs) -> Result<(), Box<dyn Error>> {
    let input = std::fs::read(&args.pdf)?;
    let password = args.password.clone().unwrap_or_default();
    let bytes = wellfriendpdf_engine::repair(input, password.as_bytes())?;
    std::fs::write(&args.output, &bytes)?;
    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "op": "repair",
                "output": args.output.display().to_string(),
                "bytes": bytes.len(),
            })
        );
    } else {
        eprintln!(
            "Repaired -> {} ({} bytes)",
            args.output.display(),
            bytes.len()
        );
    }
    Ok(())
}

fn run_linearize(args: LinearizeArgs) -> Result<(), Box<dyn Error>> {
    let engine = open_engine(&args.pdf, &args.password)?;
    let bytes = wellfriendpdf_engine::linearize(&engine)?;
    std::fs::write(&args.output, &bytes)?;
    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "op": "linearize",
                "output": args.output.display().to_string(),
                "bytes": bytes.len(),
            })
        );
    } else {
        eprintln!(
            "Linearized -> {} ({} bytes)",
            args.output.display(),
            bytes.len()
        );
    }
    Ok(())
}

fn run_codec_isolation_report(args: CodecIsolationReportArgs) -> Result<(), Box<dyn Error>> {
    let input = if let Some(path) = args.input_file {
        std::fs::read(path)?
    } else if let Some(hex) = args.input_hex {
        parse_hex_bytes_cli(&hex)?
    } else if let Some(text) = args.sample_text {
        if matches!(args.filter.as_str(), "FlateDecode" | "Fl") {
            wellfriendpdf_engine::flate_encode(text.as_bytes(), 6)
        } else {
            text.into_bytes()
        }
    } else {
        return Err(usage_error(
            "pass one of --input-file, --input-hex, or --sample-text",
        ));
    };

    let mut config =
        wellfriendpdf_engine::CodecIsolationConfig::from_policy_str(Some(&args.policy))?
            .with_timeout_ms(args.timeout_ms)
            .with_max_decoded_bytes(args.max_output_bytes);
    if let Some(worker) = args.worker {
        config = config.with_worker_path(worker);
    }
    let result = wellfriendpdf_engine::decode_filter_with_isolation(&args.filter, &input, &config);
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "schema_version": wellfriendpdf_engine::REPORT_ENVELOPE_VERSION,
            "kind": "codec_isolation_report",
            "report": result.report,
        }))?
    );
    Ok(())
}

/// Expand a split output pattern. Supports `%d` and `%0Nd` (zero-padded width
/// N) for the page number. If the pattern contains no `%`, the page number is
/// appended before the extension to avoid overwriting a single file.
fn expand_split_pattern(pattern: &str, page: usize) -> std::path::PathBuf {
    if let Some(pct) = pattern.find('%') {
        // Parse a printf-ish "%[0][width]d" directive.
        let after = &pattern[pct + 1..];
        let mut chars = after.char_indices().peekable();
        let mut zero_pad = false;
        if let Some(&(_, '0')) = chars.peek() {
            zero_pad = true;
            chars.next();
        }
        let mut width = 0usize;
        let mut consumed = 0usize;
        while let Some(&(i, c)) = chars.peek() {
            if c.is_ascii_digit() {
                width = width * 10 + (c as usize - '0' as usize);
                consumed = i + 1;
                chars.next();
            } else {
                break;
            }
        }
        // Expect a trailing 'd'.
        if let Some(&(i, 'd')) = chars.peek() {
            let directive_end = pct + 1 + i + 1;
            let num = if zero_pad {
                format!("{page:0width$}")
            } else {
                page.to_string()
            };
            let _ = consumed;
            let mut result = String::with_capacity(pattern.len() + num.len());
            result.push_str(&pattern[..pct]);
            result.push_str(&num);
            result.push_str(&pattern[directive_end..]);
            return std::path::PathBuf::from(result);
        }
    }
    // No usable directive: insert -<page> before the extension.
    let p = std::path::Path::new(pattern);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("page");
    let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("pdf");
    let parent = p.parent();
    let name = format!("{stem}-{page}.{ext}");
    match parent {
        Some(dir) if !dir.as_os_str().is_empty() => dir.join(name),
        _ => std::path::PathBuf::from(name),
    }
}

/// Parse a page selection preserving order and duplicates (e.g. "5,1,3-4,1").
/// Unlike [`parse_page_range_cli`], it does NOT sort or dedupe — extraction
/// honours the exact order the user requests. Out-of-range pages are dropped
/// with a warning so a typo doesn't silently reorder the rest.
fn parse_page_selection_ordered(spec: &str, total: usize) -> Result<Vec<usize>, Box<dyn Error>> {
    if spec.trim() == "all" || spec.trim().is_empty() {
        return Ok((1..=total).collect());
    }
    let mut pages = Vec::new();
    for part in spec.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((start, end)) = part.split_once('-') {
            let start: usize = start.trim().parse()?;
            let end: usize = end.trim().parse()?;
            if start <= end {
                for p in start..=end {
                    push_in_range(&mut pages, p, total);
                }
            } else {
                // Descending range, e.g. "9-5": honour the reverse order.
                for p in (end..=start).rev() {
                    push_in_range(&mut pages, p, total);
                }
            }
        } else {
            let p: usize = part.parse()?;
            push_in_range(&mut pages, p, total);
        }
    }
    Ok(pages)
}

fn push_in_range(pages: &mut Vec<usize>, page: usize, total: usize) {
    if (1..=total).contains(&page) {
        pages.push(page);
    } else {
        eprintln!("Warning: page {page} out of range (1..={total}); skipping");
    }
}

fn parse_page_range_cli(spec: &str, total: usize) -> Result<Vec<usize>, Box<dyn Error>> {
    if spec == "all" || spec.trim().is_empty() {
        return Ok((1..=total).collect());
    }

    let mut pages = Vec::new();
    for part in spec.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        if let Some((start, end)) = part.split_once('-') {
            let start = start.trim().parse::<usize>()?;
            let end = end.trim().parse::<usize>()?;
            if start <= end {
                for page in start..=end {
                    if (1..=total).contains(&page) {
                        pages.push(page);
                    }
                }
            }
        } else {
            let page = part.parse::<usize>()?;
            if (1..=total).contains(&page) {
                pages.push(page);
            }
        }
    }

    pages.sort_unstable();
    pages.dedup();
    Ok(pages)
}

fn parse_hex_bytes_cli(spec: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    let compact: String = spec.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    if !compact.len().is_multiple_of(2) {
        return Err(usage_error(
            "--input-hex must contain an even number of hex digits",
        ));
    }
    let mut out = Vec::with_capacity(compact.len() / 2);
    let bytes = compact.as_bytes();
    for idx in (0..bytes.len()).step_by(2) {
        let pair = std::str::from_utf8(&bytes[idx..idx + 2])?;
        out.push(u8::from_str_radix(pair, 16).map_err(|_| {
            CliError::new(
                CliExitCode::Usage,
                format!("--input-hex contains invalid hex byte '{pair}'"),
            )
        })?);
    }
    Ok(out)
}

fn parse_char_range_cli(spec: &str) -> Result<(usize, usize), Box<dyn Error>> {
    let Some((start, end)) = spec.split_once(':') else {
        return Err(usage_error("--delete-range must be formatted start:end"));
    };
    let start = start.trim().parse::<usize>()?;
    let end = end.trim().parse::<usize>()?;
    if start > end {
        return Err(usage_error("--delete-range start must be <= end"));
    }
    Ok((start, end))
}

fn parse_region_cli(spec: &str) -> Result<wellfriendpdf_engine::PageRegion, Box<dyn Error>> {
    let values: Vec<f64> = spec
        .split(',')
        .map(|part| part.trim().parse::<f64>())
        .collect::<Result<Vec<_>, _>>()?;
    if values.len() != 4 {
        return Err("region must be four comma-separated numbers: x0,y0,x1,y1".into());
    }
    wellfriendpdf_engine::PageRegion::new(values[0], values[1], values[2], values[3])
        .map_err(|err| err.into())
}

fn parse_profile_cli(
    name: &str,
) -> Result<wellfriendpdf_engine::ExtractionProfile, Box<dyn Error>> {
    wellfriendpdf_engine::ExtractionProfile::parse(name).ok_or_else(|| {
        format!(
            "unknown --profile '{name}'; use fast-text, layout-faithful, tables-focused, or rag-chunks"
        )
        .into()
    })
}

#[cfg(test)]
mod tests {
    use super::{
        certificate_input_is_pem, expand_split_pattern, parse_page_range_cli,
        parse_page_selection_ordered, parse_profile_cli, parse_region_cli, Cli, Commands,
    };
    use clap::Parser;
    use std::path::PathBuf;

    #[test]
    fn cli_page_range_parser_handles_all_formats() {
        assert_eq!(parse_page_range_cli("all", 3).unwrap(), vec![1, 2, 3]);
        assert_eq!(parse_page_range_cli("1", 5).unwrap(), vec![1]);
        assert_eq!(parse_page_range_cli("2-4", 5).unwrap(), vec![2, 3, 4]);
        assert_eq!(parse_page_range_cli("1,3,5", 5).unwrap(), vec![1, 3, 5]);
        assert_eq!(parse_page_range_cli("3-10", 5).unwrap(), vec![3, 4, 5]);
    }

    #[test]
    fn cli_region_parser_accepts_four_numbers() {
        let region = parse_region_cli("1,2,3,4").unwrap();
        assert_eq!(region.as_array(), [1.0, 2.0, 3.0, 4.0]);
        assert!(parse_region_cli("1,2,3").is_err());
    }

    #[test]
    fn cli_profile_parser_accepts_named_profiles() {
        assert_eq!(
            parse_profile_cli("layout-faithful").unwrap(),
            wellfriendpdf_engine::ExtractionProfile::LayoutFaithful
        );
        assert!(parse_profile_cli("made-up").is_err());
    }

    #[test]
    fn ordered_selection_preserves_order_and_duplicates() {
        // Order is kept, duplicates retained, ranges expanded in place.
        assert_eq!(
            parse_page_selection_ordered("5,1,3-4,1", 9).unwrap(),
            vec![5, 1, 3, 4, 1]
        );
        // Out-of-range pages are dropped (with a warning), not errors.
        assert_eq!(
            parse_page_selection_ordered("1,3,99", 5).unwrap(),
            vec![1, 3]
        );
        // Descending ranges go in reverse.
        assert_eq!(
            parse_page_selection_ordered("9-7", 9).unwrap(),
            vec![9, 8, 7]
        );
        // "all" still expands forward.
        assert_eq!(
            parse_page_selection_ordered("all", 3).unwrap(),
            vec![1, 2, 3]
        );
        // Non-contiguous subset.
        assert_eq!(
            parse_page_selection_ordered("1,3,5", 5).unwrap(),
            vec![1, 3, 5]
        );
    }

    #[test]
    fn split_pattern_expands_directives() {
        assert_eq!(
            expand_split_pattern("page-%d.pdf", 7),
            PathBuf::from("page-7.pdf")
        );
        assert_eq!(
            expand_split_pattern("out-%03d.pdf", 7),
            PathBuf::from("out-007.pdf")
        );
        assert_eq!(expand_split_pattern("p%04d", 42), PathBuf::from("p0042"));
        // No directive: page number inserted before extension.
        assert_eq!(
            expand_split_pattern("doc.pdf", 3),
            PathBuf::from("doc-3.pdf")
        );
    }

    #[test]
    fn semantic_closeout_cli_surfaces_parse_without_ambiguous_flags() {
        let semantic = Cli::try_parse_from([
            "wellfriendpdf",
            "semantic-export",
            "fixture.pdf",
            "--view",
            "search",
            "--query",
            "invoice",
            "--chunk-mode",
            "table-row",
            "--dictionary-pack",
            "dictionary.json",
        ])
        .unwrap();
        let Commands::SemanticExport(args) = semantic.command else {
            panic!("expected semantic-export command");
        };
        assert_eq!(args.view, "search");
        assert_eq!(args.query.as_deref(), Some("invoice"));
        assert_eq!(args.chunk_mode, "table-row");
        assert_eq!(args.dictionary_packs.len(), 1);

        let chunk = Cli::try_parse_from([
            "wellfriendpdf",
            "chunk",
            "fixture.pdf",
            "--advanced",
            "--mode",
            "cjk",
            "--overlap",
            "0",
        ])
        .unwrap();
        let Commands::Chunk(args) = chunk.command else {
            panic!("expected chunk command");
        };
        assert!(args.advanced);
        assert_eq!(args.mode, "cjk");
        assert_eq!(args.overlap, 0);
    }

    #[test]
    fn signature_validation_certificate_inputs_accept_pem_detection_without_misclassifying_der() {
        assert!(certificate_input_is_pem(
            b"\n-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n"
        ));
        assert!(!certificate_input_is_pem(&[0x30, 0x82, 0x01, 0x00]));
    }
}
