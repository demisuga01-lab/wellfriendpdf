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
            engine = oxide_engine::ENGINE_VERSION,
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
    name = "oxide",
    about = "Oxide — pure-Rust PDF processing tool",
    version,
    long_version = long_version(),
    after_help = "Command groups:\n  Extraction: extract-text, extract-tables, extract-fields, extract-images, parse, document-model, chunk\n  Rendering/conversion: render, pdf-to-jpg, image-to-pdf, pdf-to-xlsx, pdf-to-pptx, pdf-to-docx, xlsx-to-pdf, pptx-to-pdf, docx-to-pdf, to-html\n  Structure/editing: merge, split, extract-pages, organize, rotate, watermark, add-page-numbers, optimize, repair, linearize\n  Info/security: info, parser-report, fonts, detach, verify-sig, encrypt, decrypt, analyze, eval-score\n\nExamples:\n  oxide extract-text input.pdf --structured --format json\n  oxide parser-report input.pdf --mode audit\n  oxide pdf-to-jpg input.pdf --out-dir pages --dpi 150\n  oxide image-to-pdf img1.jpg img2.png --out combined.pdf\n  oxide pdf-to-xlsx report.pdf --out report.xlsx\n  oxide pdf-to-pptx deck.pdf --out deck.pptx\n  oxide pdf-to-docx report.pdf --out report.docx\n  oxide xlsx-to-pdf workbook.xlsx --out workbook.pdf\n  oxide watermark input.pdf --text CONFIDENTIAL --out out.pdf"
)]
struct Cli {
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
        if let Some(oxide) = error.downcast_ref::<oxide_engine::OxideError>() {
            return match oxide.kind() {
                oxide_engine::ErrorKind::Io => CliExitCode::Io,
                oxide_engine::ErrorKind::UnsupportedFeature => CliExitCode::Unsupported,
                oxide_engine::ErrorKind::MalformedPdf
                | oxide_engine::ErrorKind::Parse
                | oxide_engine::ErrorKind::MissingObject
                | oxide_engine::ErrorKind::Encrypted
                | oxide_engine::ErrorKind::ResourceLimit => CliExitCode::Input,
                oxide_engine::ErrorKind::Cancelled => CliExitCode::Internal,
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
    /// Report annotations, QuadPoints, appearances, and unsafe actions
    AnnotationsReport(AnnotationsReportArgs),
    /// Report page boxes, labels/outlines/destinations, and page-op preservation risks
    PagesReport(PagesReportArgs),
    /// Combined Prompt 07 interactive/data-layer report
    InteractiveReport(InteractiveReportArgs),
    /// Apply true redaction from search terms and/or explicit rectangles
    Redact(RedactArgs),
    /// Split a PDF into RAG-ready semantic chunks (structure-aware, token-sized,
    /// with overlap + heading context) as a JSON chunks array for embedding
    /// pipelines. Tables/figures stay intact; headings drive boundaries.
    Chunk(ChunkArgs),
    /// Score an extraction result against ground truth using standard metrics
    /// (CER/WER/reading-order/table cell-F1/TEDS/field-F1/block-type accuracy).
    /// Reads a ScoreInput JSON (file or stdin), writes a ScoreOutput JSON. The
    /// pure-Rust scoring core the extraction benchmark harness drives.
    EvalScore(EvalScoreArgs),
    /// Extract embedded images from a PDF as a ZIP
    ExtractImages(ExtractImagesArgs),
    /// Render PDF pages to images as a ZIP
    Render(RenderArgs),
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
    /// Convert a DOCX document to PDF with Oxide's native writer
    DocxToPdf(OfficeToPdfArgs),
    /// Convert an XLSX workbook to PDF with Oxide's native writer
    XlsxToPdf(OfficeToPdfArgs),
    /// Convert a PPTX presentation to PDF with Oxide's native writer
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
    /// Emit structured parser diagnostics, repair/audit status, and source metrics
    ParserReport(ParserReportArgs),
    /// List the fonts used in a PDF (pdffonts-equivalent)
    Fonts(FontsArgs),
    /// List or extract embedded file attachments (pdfdetach-equivalent)
    Detach(DetachArgs),
    /// Convert a PDF to HTML or XML (pdftohtml-equivalent)
    ToHtml(ToHtmlArgs),
    /// Verify digital signatures in a PDF (pdfsig-equivalent)
    VerifySig(VerifySigArgs),
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
    /// Algorithm: aes256 (default), aes128, or rc4
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
    /// (Prompt 06 geometry/provenance model). Ignored without either flag.
    #[arg(long, default_value = "text")]
    format: String,
    /// Include detailed structure attachment in model-json output.
    #[arg(long)]
    include_structure: bool,
    /// Include detailed char/span provenance in model-json output.
    #[arg(long)]
    include_provenance: bool,
    /// CJK tokenization for model-json: char, simple, or dictionary (dictionary
    /// currently aliases the bounded simple segmenter unless an API caller
    /// supplies a dictionary in a later phase).
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
    /// Emit a JSON result summary.
    #[arg(long)]
    json: bool,
    /// Fail if verification finds any requested term after redaction.
    #[arg(long)]
    strict: bool,
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
    /// OXIDE_MAX_RENDER_PIXELS environment variable when set.
    #[arg(long)]
    max_render_pixels: Option<u64>,
    /// Emit a JSON result summary
    #[arg(long)]
    json: bool,
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
    /// Password for an encrypted PDF (the empty user password is tried automatically)
    #[arg(long)]
    password: Option<String>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let default_panic_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| dispatch(cli)));
    std::panic::set_hook(default_panic_hook);

    match result {
        Ok(Ok(())) => ExitCode::from(CliExitCode::Success.code()),
        Ok(Err(err)) => {
            let code = classify_error(err.as_ref());
            eprintln!("oxide: {}: {}", code.label(), err);
            ExitCode::from(code.code())
        }
        Err(_) => {
            eprintln!(
                "oxide: internal error: command panicked; this is a bug, not a PDF-level error"
            );
            ExitCode::from(CliExitCode::Internal.code())
        }
    }
}

fn dispatch(cli: Cli) -> Result<(), Box<dyn Error>> {
    match cli.command {
        Commands::ExtractText(args) => run_extract_text(args),
        Commands::ExtractTables(args) => run_extract_tables(args),
        Commands::Parse(args) => run_parse(args),
        Commands::DocumentModel(args) => run_document_model(args),
        Commands::ExtractFields(args) => run_extract_fields(args),
        Commands::FormsReport(args) => run_forms_report(args),
        Commands::AnnotationsReport(args) => run_annotations_report(args),
        Commands::PagesReport(args) => run_pages_report(args),
        Commands::InteractiveReport(args) => run_interactive_report(args),
        Commands::Redact(args) => run_redact(args),
        Commands::Chunk(args) => run_chunk(args),
        Commands::EvalScore(args) => run_eval_score(args),
        Commands::ExtractImages(args) => run_extract_images(args),
        Commands::Render(args) => run_render(args),
        Commands::PdfToJpg(args) => run_pdf_to_jpg(args),
        Commands::ImageToPdf(args) => run_image_to_pdf(args),
        Commands::PdfToXlsx(args) => run_pdf_to_xlsx(args),
        Commands::PdfToPptx(args) => run_pdf_to_pptx(args),
        Commands::PdfToDocx(args) => run_pdf_to_docx(args),
        Commands::DocxToPdf(args) => run_docx_to_pdf(args),
        Commands::XlsxToPdf(args) => run_xlsx_to_pdf(args),
        Commands::PptxToPdf(args) => run_pptx_to_pdf(args),
        Commands::Analyze(args) => run_analyze(args),
        Commands::Merge(args) => run_merge(args),
        Commands::Split(args) => run_split(args),
        Commands::ExtractPages(args) => run_extract_pages(args),
        Commands::Info(args) => run_info(args),
        Commands::ParserReport(args) => run_parser_report(args),
        Commands::Fonts(args) => run_fonts(args),
        Commands::Detach(args) => run_detach(args),
        Commands::ToHtml(args) => run_to_html(args),
        Commands::VerifySig(args) => run_verify_sig(args),
        Commands::Encrypt(args) => run_encrypt(args),
        Commands::Decrypt(args) => run_decrypt(args),
        Commands::Watermark(args) => run_watermark(args),
        Commands::AddPageNumbers(args) => run_page_numbers(args),
        Commands::Organize(args) => run_organize(args),
        Commands::Rotate(args) => run_rotate(args),
        Commands::Optimize(args) => run_optimize(args),
        Commands::Repair(args) => run_repair(args),
        Commands::Linearize(args) => run_linearize(args),
    }
}

fn run_extract_text(args: ExtractTextArgs) -> Result<(), Box<dyn Error>> {
    use rayon::prelude::*;

    let engine = match &args.password {
        Some(password) => {
            oxide_engine::ContentEngine::open_path_with_password(&args.pdf, password.as_bytes())?
        }
        None => oxide_engine::ContentEngine::open_path(&args.pdf)?,
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
    let parallel_window = oxide_engine::bounded_text_parallel_window(page_nums.len());
    let page_texts: Vec<oxide_engine::Result<String>> = if parallel_window >= 4 {
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
    engine: &oxide_engine::ContentEngine,
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
    engine: &oxide_engine::ContentEngine,
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
) -> Result<oxide_engine::TextSemanticOptions, Box<dyn Error>> {
    let cjk_segmentation = match args.cjk_segmentation.to_ascii_lowercase().as_str() {
        "char" => oxide_engine::CjkSegmentationMode::Char,
        "simple" => oxide_engine::CjkSegmentationMode::Simple,
        "dictionary" | "dict" => oxide_engine::CjkSegmentationMode::Dictionary,
        other => {
            return Err(usage_error(format!(
                "unknown --cjk-segmentation '{other}'; use char, simple, or dictionary"
            )));
        }
    };
    let defaults = oxide_engine::TextSemanticOptions::default();
    Ok(oxide_engine::TextSemanticOptions {
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
    engine: &oxide_engine::ContentEngine,
    page_nums: Vec<usize>,
    args: &ExtractTextArgs,
    policy: oxide_engine::OcrPolicy,
) -> Result<(), Box<dyn Error>> {
    use oxide_engine::{BlockKind, ParseOptions};

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
        Some(password) => {
            oxide_engine::ContentEngine::open_path_with_password(&args.pdf, password.as_bytes())?
        }
        None => oxide_engine::ContentEngine::open_path(&args.pdf)?,
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

fn table_pages_to_html(pages: &[(usize, Vec<oxide_engine::analysis::tables::Table>)]) -> String {
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
fn build_ocr_engine() -> Result<std::sync::Arc<dyn oxide_engine::OcrEngine>, Box<dyn Error>> {
    let engine = oxide_ocr_tesseract::TesseractEngine::new()?;
    Ok(std::sync::Arc::new(engine))
}

#[cfg(not(feature = "ocr"))]
fn build_ocr_engine() -> Result<std::sync::Arc<dyn oxide_engine::OcrEngine>, Box<dyn Error>> {
    Err(unsupported_error(
        "this build of oxide has no OCR backend; rebuild the CLI with \
         `--features ocr` (and install the `tesseract` binary + language data) \
         to use --ocr",
    ))
}

/// Build [`oxide_engine::OcrOptions`] from the shared CLI `--ocr-lang`/`--ocr-dpi`
/// flags (languages split on `+`/`,`, falling back to `eng`). Used by every
/// command that supports `--ocr` so the option parsing stays identical.
fn ocr_options(ocr_lang: &str, ocr_dpi: u32) -> oxide_engine::OcrOptions {
    let langs: Vec<String> = ocr_lang
        .split(['+', ','])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    oxide_engine::OcrOptions {
        languages: if langs.is_empty() {
            vec!["eng".to_string()]
        } else {
            langs
        },
        dpi: ocr_dpi,
        psm: None,
    }
}

/// Interpret the CLI `--ocr` flag value into an [`oxide_engine::OcrPolicy`].
///
/// `--ocr` is an optional-value flag: absent → `None` (OCR off, the default);
/// bare `--ocr` → `Some("auto")` via clap's `default_missing_value`; `--ocr off`
/// / `auto` / `force` map to the matching policy. Returns `Ok(None)` when OCR is
/// off (the caller skips the OCR path), `Ok(Some(policy))` otherwise, or an
/// error for an unrecognized token.
fn ocr_policy_from_flag(
    flag: &Option<String>,
) -> Result<Option<oxide_engine::OcrPolicy>, Box<dyn Error>> {
    match flag.as_deref() {
        None => Ok(None),
        Some(tok) => match oxide_engine::OcrPolicy::parse(tok) {
            Some(oxide_engine::OcrPolicy::Off) => Ok(None),
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
    use oxide_engine::{ImageHandling, ParseOptions, SerializeOptions};

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
        .filter(|p| p.source == oxide_engine::PageSource::Scanned)
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
    use oxide_engine::{DocType, ExtractOptions};

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
    let output = serde_json::to_string_pretty(&oxide_engine::forms_report(&engine)?)?;
    write_output_optional(&args.output, &output)?;
    Ok(())
}

fn run_annotations_report(args: AnnotationsReportArgs) -> Result<(), Box<dyn Error>> {
    let engine = open_engine(&args.pdf, &args.password)?;
    let output = serde_json::to_string_pretty(&oxide_engine::annotation_report(&engine)?)?;
    write_output_optional(&args.output, &output)?;
    Ok(())
}

fn run_pages_report(args: PagesReportArgs) -> Result<(), Box<dyn Error>> {
    let engine = open_engine(&args.pdf, &args.password)?;
    let output = serde_json::to_string_pretty(&oxide_engine::page_operations_report(&engine)?)?;
    write_output_optional(&args.output, &output)?;
    Ok(())
}

fn run_interactive_report(args: InteractiveReportArgs) -> Result<(), Box<dyn Error>> {
    let engine = open_engine(&args.pdf, &args.password)?;
    let output = serde_json::to_string_pretty(&oxide_engine::interactive_report(&engine)?)?;
    write_output_optional(&args.output, &output)?;
    Ok(())
}

fn run_redact(args: RedactArgs) -> Result<(), Box<dyn Error>> {
    if args.text.is_empty() && args.rects.is_empty() {
        return Err("redact requires at least one --text or --rect".into());
    }
    let input = read_edit_input(&args.pdf, &args.password)?;
    let engine = oxide_engine::ContentEngine::open_bytes(input.clone())?;
    let total = engine.page_count()?;
    let search_pages = parse_page_range_cli(&args.pages, total)?;
    let mut explicit_regions = Vec::new();
    for spec in &args.rects {
        explicit_regions.push(parse_redact_rect_cli(spec, total)?);
    }

    let mut editor = oxide_engine::PdfEditor::open_bytes(input)?;
    let redaction_options = oxide_engine::RedactionOptions {
        fill: oxide_engine::Color::black(),
        scrub_metadata: !args.no_metadata_scrub,
    };
    let mut search_regions = Vec::new();
    for term in &args.text {
        if term.trim().is_empty() {
            continue;
        }
        let matches = engine.search_text(
            &search_pages,
            term,
            oxide_engine::TextSearchOptions {
                case_sensitive: false,
                include_hidden: true,
                ..oxide_engine::TextSearchOptions::default()
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

    let bytes = editor.save_to_bytes(oxide_engine::EditMode::FullRewrite)?;
    let verification = oxide_engine::redaction_verification_report(&bytes, &args.text)?;
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
    use oxide_engine::{ChunkOptions, ParseOptions};

    if !matches!(args.format.to_lowercase().as_str(), "json") {
        return Err(format!("unknown --format '{}'; only json is supported", args.format).into());
    }

    let engine = open_engine(&args.pdf, &args.password)?;
    let total = engine.page_count()?;
    let page_nums = parse_page_range_cli(&args.pages, total)?;

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
    let output_json =
        oxide_engine::score_json(&input_json).map_err(|e| -> Box<dyn Error> { e.into() })?;
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
        oxide_engine::render_document_markdown(&model)
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
    use oxide_engine::{ImageLocateOptions, ImageLocator, ImageOutputFormat};
    use std::io::Write;
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

    let images: Vec<oxide_engine::PlacedImageReference> = if let Some(region) = region {
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
            .map(|image| oxide_engine::PlacedImageReference {
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

fn run_render(args: RenderArgs) -> Result<(), Box<dyn Error>> {
    use oxide_engine::{ImageEncoder, ImageOutputFormat, RenderMode};
    use rayon::prelude::*;
    use std::io::Write;
    use zip::{write::FileOptions, CompressionMethod, ZipWriter};

    let dpi = args.dpi.clamp(24, 600);
    if dpi != args.dpi {
        eprintln!("Warning: DPI clamped to {} (valid range: 24-600)", dpi);
    }

    // Honor an explicit per-page pixel cap by exporting it for the engine's
    // `max_render_pixels()` resolver (also read by the svg/ps/eps sub-paths,
    // which all size their pages through `page_viewport`).
    if let Some(cap) = args.max_render_pixels {
        std::env::set_var("OXIDE_MAX_RENDER_PIXELS", cap.to_string());
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
    let format = oxide_engine::RasterImageFormat::parse(&args.format)
        .ok_or_else(|| format!("unknown --format '{}'; use jpg or png", args.format))?;
    let results = oxide_engine::export_pdf_pages_to_images(
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
    let page_size = oxide_engine::ImagePdfPageSize::parse(&args.page_size).ok_or_else(|| {
        format!(
            "unknown --page-size '{}'; use a4, letter, or size-to-image",
            args.page_size
        )
    })?;
    let bytes = oxide_engine::images_to_pdf_from_paths(
        &args.images,
        oxide_engine::ImageToPdfOptions {
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
    let layout = oxide_engine::XlsxLayout::parse(&args.layout)
        .ok_or_else(|| format!("unknown --layout '{}'; use pages or tables", args.layout))?;
    let bytes = oxide_engine::pdf_to_xlsx(&engine, &oxide_engine::XlsxOptions { layout })?;
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
    let options = oxide_engine::PptxOptions {
        include_images: !args.no_images,
    };
    let bytes = oxide_engine::pdf_to_pptx(&engine, &options)?;
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
    let options = oxide_engine::DocxOptions {
        include_images: !args.no_images,
    };
    let bytes = oxide_engine::pdf_to_docx(&engine, &options)?;
    std::fs::write(&args.output, &bytes)?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "op": "pdf-to-docx",
                "input": args.pdf,
                "output": args.output,
                "include_images": options.include_images,
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

fn run_docx_to_pdf(args: OfficeToPdfArgs) -> Result<(), Box<dyn Error>> {
    run_office_to_pdf(args, "docx-to-pdf", oxide_engine::docx_to_pdf)
}

fn run_xlsx_to_pdf(args: OfficeToPdfArgs) -> Result<(), Box<dyn Error>> {
    run_office_to_pdf(args, "xlsx-to-pdf", oxide_engine::xlsx_to_pdf)
}

fn run_pptx_to_pdf(args: OfficeToPdfArgs) -> Result<(), Box<dyn Error>> {
    run_office_to_pdf(args, "pptx-to-pdf", oxide_engine::pptx_to_pdf)
}

fn run_office_to_pdf(
    args: OfficeToPdfArgs,
    op: &str,
    convert: fn(&[u8], &oxide_engine::OfficeToPdfOptions) -> oxide_engine::Result<Vec<u8>>,
) -> Result<(), Box<dyn Error>> {
    let input = std::fs::read(&args.input)?;
    let bytes = convert(&input, &oxide_engine::OfficeToPdfOptions::default())?;
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
    use oxide_engine::{ContentEngine, PdfAnalyzer};

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
) -> Result<oxide_engine::ContentEngine, Box<dyn Error>> {
    use oxide_engine::ContentEngine;
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
        Ok(oxide_engine::decrypt_pdf(&engine)?)
    } else {
        Ok(std::fs::read(pdf)?)
    }
}

fn parse_stamp_position(value: &str) -> Result<oxide_engine::StampPosition, Box<dyn Error>> {
    oxide_engine::StampPosition::parse(value)
        .ok_or_else(|| format!("unknown position '{value}'").into())
}

fn parse_rgb_color(value: &str) -> Result<oxide_engine::RgbColor, Box<dyn Error>> {
    let hex = value.strip_prefix('#').unwrap_or(value);
    if hex.len() != 6 {
        return Err(format!("color '{value}' must be #RRGGBB").into());
    }
    let r = u8::from_str_radix(&hex[0..2], 16)?;
    let g = u8::from_str_radix(&hex[2..4], 16)?;
    let b = u8::from_str_radix(&hex[4..6], 16)?;
    Ok(oxide_engine::RgbColor {
        r: f64::from(r) / 255.0,
        g: f64::from(g) / 255.0,
        b: f64::from(b) / 255.0,
    })
}

#[derive(Debug, Clone, Copy)]
struct RedactRegionSpec {
    page: usize,
    rect: oxide_engine::ImageRect,
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
        rect: oxide_engine::ImageRect::new(values[0], values[1], values[2], values[3]),
    })
}

fn redaction_rect_from_quads(quads: &[oxide_engine::TextQuad]) -> Option<oxide_engine::ImageRect> {
    let bbox = oxide_engine::TextQuad::union(quads)?;
    let pad = 0.5;
    Some(oxide_engine::ImageRect::new(
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

fn run_parser_report(args: ParserReportArgs) -> Result<(), Box<dyn Error>> {
    let mode = match args.mode.to_ascii_lowercase().as_str() {
        "strict" => oxide_engine::ParserMode::Strict,
        "repair" => oxide_engine::ParserMode::Repair,
        "audit" => oxide_engine::ParserMode::Audit,
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
    let mut report = oxide_engine::parser_report_bytes_with_options(
        &bytes,
        mode,
        password,
        oxide_engine::ParserReportOptions {
            include_decode: args.include_decode,
            decode_limits,
        },
    );
    if let Some(max) = args.max_diagnostics {
        report.diagnostics.truncate(max);
    }
    let color_report = if args.include_color {
        Some(oxide_engine::color_report_bytes(
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
) -> Result<oxide_engine::ColorValidationProfile, Box<dyn Error>> {
    match profile.to_ascii_lowercase().as_str() {
        "generic" | "default" => Ok(oxide_engine::ColorValidationProfile::Generic),
        "pdfa" | "pdf/a" | "pdf-a" => Ok(oxide_engine::ColorValidationProfile::PdfA),
        "pdfx" | "pdf/x" | "pdf-x" => Ok(oxide_engine::ColorValidationProfile::PdfX),
        other => Err(format!(
            "unknown --color-profile value '{other}'; use generic, pdfa, or pdfx"
        )
        .into()),
    }
}

fn parser_report_decode_limits(
    args: &ParserReportArgs,
) -> Result<oxide_engine::DecodeLimits, Box<dyn Error>> {
    let mut limits = match args.decode_profile.to_ascii_lowercase().as_str() {
        "default" => oxide_engine::DecodeLimits::default(),
        "low-memory" | "low_memory" => oxide_engine::DecodeLimits::strict_low_memory(),
        "audit" => oxide_engine::DecodeLimits::audit_generous(),
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
    Ok(limits)
}

fn enforce_parser_report_fail_on(
    fail_on: &str,
    report: &oxide_engine::ParserReport,
) -> Result<(), Box<dyn Error>> {
    let should_fail = match fail_on.to_ascii_lowercase().as_str() {
        "never" => false,
        "fatal" => report.diagnostics.iter().any(|diagnostic| {
            matches!(
                diagnostic.severity,
                oxide_engine::ParserSeverity::FatalError
                    | oxide_engine::ParserSeverity::SecurityLimit
            )
        }),
        "error" => report.diagnostics.iter().any(|diagnostic| {
            matches!(
                diagnostic.severity,
                oxide_engine::ParserSeverity::RecoverableError
                    | oxide_engine::ParserSeverity::FatalError
                    | oxide_engine::ParserSeverity::SecurityLimit
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

fn format_parser_report_human(path: &Path, report: &oxide_engine::ParserReport) -> String {
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

fn page_size_label(s: &oxide_engine::PageSize) -> String {
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
    use oxide_engine::sanitize_filename;

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
    let to_save: Vec<&oxide_engine::Attachment> = if args.save_all {
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
    use oxide_engine::{HtmlMode, HtmlOptions};

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

fn run_verify_sig(args: VerifySigArgs) -> Result<(), Box<dyn Error>> {
    use oxide_engine::{
        Coverage, PadesLevel, RevocationStatus, SignatureStatus, SignatureTrust, SignatureValidity,
    };

    let engine = open_engine(&args.pdf, &args.password)?;
    let reports = engine.verify_signatures()?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&reports)?);
        return Ok(());
    }

    if reports.is_empty() {
        println!("No digital signatures found.");
        return Ok(());
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
    Ok(())
}

fn run_merge(args: MergeArgs) -> Result<(), Box<dyn Error>> {
    use oxide_engine::{build_merged, ContentEngine};

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
    use oxide_engine::ContentEngine;

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
    use oxide_engine::ContentEngine;

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
    use oxide_engine::crypto::{secret_bytes, EncryptAlgorithm, EncryptParams};

    let algo = EncryptAlgorithm::parse(&args.algo)
        .ok_or_else(|| format!("unknown --algo '{}'; use aes256, aes128, or rc4", args.algo))?;
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
    if !matches!(algo, EncryptAlgorithm::Aes256) {
        eprintln!(
            "Warning: {} is a legacy algorithm. Oxide reads its own output, but \
             cross-reader interop is only verified for AES-256 (the default). \
             Prefer --algo aes256 unless a consumer requires legacy encryption.",
            args.algo
        );
    }
    let bytes = oxide_engine::encrypt(&engine, &params)?;
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
    let bytes = oxide_engine::decrypt_pdf(&engine)?;
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
    let engine = oxide_engine::ContentEngine::open_bytes(input.clone())?;
    let total = engine.page_count()?;
    let pages = parse_page_range_cli(&args.pages, total)?;
    let position = parse_stamp_position(&args.position)?;
    let bytes = if let Some(text) = args.text {
        oxide_engine::watermark_text_pdf(
            input,
            &text,
            oxide_engine::TextWatermarkOptions {
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
        oxide_engine::watermark_image_pdf(
            input,
            &image,
            image_path.extension().and_then(|s| s.to_str()),
            oxide_engine::ImageWatermarkOptions {
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
    let engine = oxide_engine::ContentEngine::open_bytes(input.clone())?;
    let total = engine.page_count()?;
    let pages = parse_page_range_cli(&args.pages, total)?;
    let bytes = oxide_engine::add_page_numbers_pdf(
        input,
        oxide_engine::PageNumberOptions {
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
        oxide_engine::organize_pdf_with_insert(
            &engine,
            &order,
            Some((&inserted, insert_pages, args.insert_at)),
        )?
    } else {
        oxide_engine::organize_pdf(&engine, &order)?
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
    use oxide_engine::Rotation;

    let engine = open_engine(&args.pdf, &args.password)?;
    let total = engine.page_count()?;
    let pages = parse_page_range_cli(&args.pages, total)?;
    let rotation = if args.relative {
        Rotation::Relative(args.angle)
    } else {
        Rotation::Absolute(args.angle)
    };
    let bytes = oxide_engine::rotate_pages(&engine, &pages, rotation)?;
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

fn run_optimize(args: OptimizeArgs) -> Result<(), Box<dyn Error>> {
    let input_size = std::fs::metadata(&args.pdf)
        .map(|m| m.len() as usize)
        .unwrap_or(0);
    let engine = open_engine(&args.pdf, &args.password)?;
    let (bytes, report) = oxide_engine::optimize(&engine)?;
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
    let bytes = oxide_engine::repair(input, password.as_bytes())?;
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
    let bytes = oxide_engine::linearize(&engine)?;
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

fn parse_region_cli(spec: &str) -> Result<oxide_engine::PageRegion, Box<dyn Error>> {
    let values: Vec<f64> = spec
        .split(',')
        .map(|part| part.trim().parse::<f64>())
        .collect::<Result<Vec<_>, _>>()?;
    if values.len() != 4 {
        return Err("region must be four comma-separated numbers: x0,y0,x1,y1".into());
    }
    oxide_engine::PageRegion::new(values[0], values[1], values[2], values[3])
        .map_err(|err| err.into())
}

fn parse_profile_cli(name: &str) -> Result<oxide_engine::ExtractionProfile, Box<dyn Error>> {
    oxide_engine::ExtractionProfile::parse(name).ok_or_else(|| {
        format!(
            "unknown --profile '{name}'; use fast-text, layout-faithful, tables-focused, or rag-chunks"
        )
        .into()
    })
}

#[cfg(test)]
mod tests {
    use super::{
        expand_split_pattern, parse_page_range_cli, parse_page_selection_ordered,
        parse_profile_cli, parse_region_cli,
    };
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
            oxide_engine::ExtractionProfile::LayoutFaithful
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
}
