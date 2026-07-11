//! Office-format export helpers built from the canonical parsed document model.
//!
//! These writers deliberately consume [`crate::parse::Document`] rather than
//! running a second extraction pass. XLSX uses the document's table blocks as a
//! grid projection; PPTX uses the same page/block geometry as positioned slide
//! shapes.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{Cursor, Read, Seek, Write};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::analysis::tables::Table;
use crate::authoring::{
    FlowDocument, ImageHandle, Margins, PageSize, ParagraphStyle, PdfBuilder, TableBuilder,
    TableColumn, TextStyle,
};
use crate::editable::{EditableBuildOptions, EditableDocument};
use crate::engine::{ContentEngine, ExtractionProfile};
use crate::error::{OxideError, Result};
use crate::images::encoder::ImageOutputFormat;
use crate::parse::{Block, BlockKind, Document, InlineSpan, InlineText, ParseOptions};
use crate::versioning::resource_digest;
use crate::PageRegion;

const XLSX_MAIN_NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const REL_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PACKAGE_REL_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const PPT_NS: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const DRAWING_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const WORD_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const WP_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing";
const PIC_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/picture";
const DOCX_REL_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const WPS_NS: &str = "http://schemas.microsoft.com/office/word/2010/wordprocessingShape";

/// How detected table content is arranged in a generated XLSX workbook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XlsxLayout {
    /// One worksheet per PDF page. Non-table text remains near the tables in
    /// recovered reading order. This is the default because it preserves page
    /// provenance and is stable for mixed content.
    Pages,
    /// One worksheet per detected table, with a final `Notes` worksheet for
    /// non-tabular text. Useful for data-first PDFs with many small tables.
    Tables,
}

impl XlsxLayout {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "pages" | "page" => Some(Self::Pages),
            "tables" | "table" => Some(Self::Tables),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pages => "pages",
            Self::Tables => "tables",
        }
    }
}

/// Options for PDF-to-XLSX export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XlsxOptions {
    pub layout: XlsxLayout,
}

impl Default for XlsxOptions {
    fn default() -> Self {
        Self {
            layout: XlsxLayout::Pages,
        }
    }
}

/// Options for PDF-to-PPTX export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PptxOptions {
    /// When true, image XObjects are exported as picture shapes when they can be
    /// decoded. Decode failures are contained to that image; text/table export
    /// continues.
    pub include_images: bool,
}

impl Default for PptxOptions {
    fn default() -> Self {
        Self {
            include_images: true,
        }
    }
}

/// Options for PDF-to-DOCX export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DocxOptions {
    /// Include decodable PDF image XObjects as inline DOCX pictures.
    pub include_images: bool,
    pub layout: DocxLayout,
}

impl Default for DocxOptions {
    fn default() -> Self {
        Self {
            include_images: true,
            layout: DocxLayout::Flowing,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocxLayout {
    Flowing,
    PageFaithful,
    Hybrid,
}

impl DocxLayout {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "flowing" | "flow" => Some(Self::Flowing),
            "page-faithful" | "page_faithful" | "faithful" | "positioned" => {
                Some(Self::PageFaithful)
            }
            "hybrid" => Some(Self::Hybrid),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Flowing => "flowing",
            Self::PageFaithful => "page-faithful",
            Self::Hybrid => "hybrid",
        }
    }
}

/// Shared options for native Office-to-PDF conversion.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OfficeToPdfOptions {
    pub page_size: PageSize,
    pub margins: Margins,
}

impl Default for OfficeToPdfOptions {
    fn default() -> Self {
        Self {
            page_size: PageSize::LETTER,
            margins: Margins::all(54.0),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum OfficeBlock {
    Paragraph {
        spans: Vec<InlineSpan>,
        style: ParagraphRole,
    },
    List {
        ordered: bool,
        items: Vec<String>,
    },
    Table(Vec<Vec<String>>),
    Image {
        bytes: Vec<u8>,
        extension: String,
        width_points: f64,
        height_points: f64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParagraphRole {
    Normal,
    Title,
    Heading(u8),
}

#[derive(Debug, Clone)]
struct XlsxCell {
    col: usize,
    text: String,
    style: u8,
}

#[derive(Debug, Default)]
struct XlsxSheet {
    name: String,
    rows: BTreeMap<usize, Vec<XlsxCell>>,
    merges: Vec<String>,
    col_widths: BTreeMap<usize, f64>,
}

impl XlsxSheet {
    fn new(name: impl Into<String>) -> Self {
        Self {
            name: sanitize_sheet_name(&name.into()),
            ..Default::default()
        }
    }

    fn add_text(&mut self, row: usize, col: usize, text: impl Into<String>, style: u8) {
        let text = text.into();
        if text.trim().is_empty() {
            return;
        }
        let width = text.chars().count().clamp(8, 80) as f64 + 2.0;
        self.col_widths
            .entry(col)
            .and_modify(|w| *w = (*w).max(width))
            .or_insert(width);
        self.rows
            .entry(row)
            .or_default()
            .push(XlsxCell { col, text, style });
    }

    fn add_table(&mut self, start_row: usize, start_col: usize, table: &Table) -> usize {
        let mut max_row = start_row;
        let cells: Vec<_> = if table.cells.is_empty() {
            table
                .rows
                .iter()
                .enumerate()
                .flat_map(|(row, values)| {
                    values.iter().enumerate().map(move |(col, text)| {
                        (
                            row,
                            col,
                            1usize,
                            1usize,
                            text.clone(),
                            row == 0 && !text.trim().is_empty(),
                        )
                    })
                })
                .collect()
        } else {
            table
                .cells
                .iter()
                .map(|cell| {
                    (
                        cell.row,
                        cell.col,
                        cell.rowspan.max(1),
                        cell.colspan.max(1),
                        cell.text.clone(),
                        cell.is_header,
                    )
                })
                .collect()
        };

        for (row, col, rowspan, colspan, text, is_header) in cells {
            let out_row = start_row + row;
            let out_col = start_col + col;
            max_row = max_row.max(out_row + rowspan.saturating_sub(1));
            self.add_text(out_row, out_col, text, if is_header { 1 } else { 0 });
            if rowspan > 1 || colspan > 1 {
                let first = cell_ref(out_row, out_col);
                let last = cell_ref(out_row + rowspan - 1, out_col + colspan - 1);
                self.merges.push(format!("{first}:{last}"));
            }
        }
        max_row + 1
    }

    fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// Convert a PDF to XLSX bytes using the canonical parse model.
pub fn pdf_to_xlsx(engine: &ContentEngine, options: &XlsxOptions) -> Result<Vec<u8>> {
    let document = parse_for_office(engine, ExtractionProfile::TablesFocused)?;
    let sheets = match options.layout {
        XlsxLayout::Pages => xlsx_sheets_by_page(&document),
        XlsxLayout::Tables => xlsx_sheets_by_table(&document),
    };
    write_xlsx(sheets)
}

/// Convert a PDF to PPTX bytes using the canonical parse model.
pub fn pdf_to_pptx(engine: &ContentEngine, options: &PptxOptions) -> Result<Vec<u8>> {
    let document = parse_for_office(engine, ExtractionProfile::LayoutFaithful)?;
    write_pptx(engine, &document, options)
}

/// Convert a PDF to DOCX bytes using the canonical parse model.
pub fn pdf_to_docx(engine: &ContentEngine, options: &DocxOptions) -> Result<Vec<u8>> {
    let document = parse_for_office(engine, ExtractionProfile::LayoutFaithful)?;
    write_docx(engine, &document, options)
}

/// Convert DOCX bytes to PDF using Oxide's native authoring path.
pub fn docx_to_pdf(bytes: &[u8], options: &OfficeToPdfOptions) -> Result<Vec<u8>> {
    let blocks = parse_docx_blocks(bytes)?;
    office_blocks_to_pdf(&blocks, options)
}

/// Convert XLSX bytes to PDF using Oxide's native authoring path.
pub fn xlsx_to_pdf(bytes: &[u8], options: &OfficeToPdfOptions) -> Result<Vec<u8>> {
    let sheets = parse_xlsx_sheets(bytes)?;
    xlsx_sheets_to_pdf(&sheets, options)
}

/// Convert PPTX bytes to PDF using Oxide's native authoring path.
pub fn pptx_to_pdf(bytes: &[u8], _options: &OfficeToPdfOptions) -> Result<Vec<u8>> {
    let slides = parse_pptx_slides(bytes)?;
    pptx_slides_to_pdf(&slides)
}

fn parse_for_office(engine: &ContentEngine, profile: ExtractionProfile) -> Result<Document> {
    let document = engine.parse_document_with_profile(profile, &ParseOptions::default())?;
    let editable =
        EditableDocument::from_parse_document(engine, document, &EditableBuildOptions::default());
    Ok(editable.to_parse_document())
}

fn xlsx_sheets_by_page(document: &Document) -> Vec<XlsxSheet> {
    let mut sheets = Vec::new();
    for page in &document.pages {
        let mut sheet = XlsxSheet::new(format!("Page {}", page.number));
        let mut row = 1usize;
        let mut blocks = page_blocks(document, page.number);
        blocks.sort_by_key(|block| block.reading_order);
        for block in blocks {
            match &block.kind {
                BlockKind::Table { table, .. } => {
                    row = sheet.add_table(row, 1, table) + 1;
                }
                _ => {
                    if let Some(text) = block_text(block) {
                        let style = match block.kind {
                            BlockKind::Title { .. } | BlockKind::Heading { .. } => 1,
                            _ => 0,
                        };
                        sheet.add_text(row, 1, text, style);
                        row += 1;
                    }
                }
            }
        }
        if sheet.is_empty() {
            sheet.add_text(
                1,
                1,
                format!("Page {} contained no extracted text", page.number),
                0,
            );
        }
        sheets.push(sheet);
    }
    if sheets.is_empty() {
        let mut sheet = XlsxSheet::new("Document");
        sheet.add_text(1, 1, "No extracted content", 0);
        sheets.push(sheet);
    }
    sheets
}

fn xlsx_sheets_by_table(document: &Document) -> Vec<XlsxSheet> {
    let mut sheets = Vec::new();
    let mut notes = XlsxSheet::new("Notes");
    let mut notes_row = 1usize;
    let mut table_index = 1usize;
    for block in &document.body {
        match &block.kind {
            BlockKind::Table { table, .. } => {
                let mut sheet = XlsxSheet::new(format!("P{} Table {}", block.page, table_index));
                sheet.add_table(1, 1, table);
                sheets.push(sheet);
                table_index += 1;
            }
            _ => {
                if let Some(text) = block_text(block) {
                    notes.add_text(notes_row, 1, format!("Page {}", block.page), 1);
                    notes.add_text(notes_row, 2, text, 0);
                    notes_row += 1;
                }
            }
        }
    }
    if !notes.is_empty() {
        sheets.push(notes);
    }
    if sheets.is_empty() {
        let mut sheet = XlsxSheet::new("Document");
        sheet.add_text(1, 1, "No extracted content", 0);
        sheets.push(sheet);
    }
    sheets
}

fn write_xlsx(sheets: Vec<XlsxSheet>) -> Result<Vec<u8>> {
    let mut cursor = Cursor::new(Vec::new());
    let mut zip = ZipWriter::new(&mut cursor);
    let opts = deterministic_zip_options();

    zip_file(
        &mut zip,
        opts,
        "[Content_Types].xml",
        &xlsx_content_types(sheets.len()),
    )?;
    zip_file(&mut zip, opts, "_rels/.rels", PACKAGE_RELS_XLSX)?;
    zip_file(&mut zip, opts, "xl/workbook.xml", &xlsx_workbook(&sheets))?;
    zip_file(
        &mut zip,
        opts,
        "xl/_rels/workbook.xml.rels",
        &xlsx_workbook_rels(sheets.len()),
    )?;
    zip_file(&mut zip, opts, "xl/styles.xml", XLSX_STYLES)?;
    for (idx, sheet) in sheets.iter().enumerate() {
        zip_file(
            &mut zip,
            opts,
            &format!("xl/worksheets/sheet{}.xml", idx + 1),
            &xlsx_sheet_xml(sheet),
        )?;
    }
    zip.finish().map_err(zip_err)?;
    Ok(cursor.into_inner())
}

fn zip_file<W: Write + Seek>(
    zip: &mut ZipWriter<W>,
    opts: SimpleFileOptions,
    name: &str,
    contents: &str,
) -> Result<()> {
    zip.start_file(name, opts).map_err(zip_err)?;
    zip.write_all(contents.as_bytes())?;
    Ok(())
}

fn zip_bytes<W: Write + Seek>(
    zip: &mut ZipWriter<W>,
    opts: SimpleFileOptions,
    name: &str,
    contents: &[u8],
) -> Result<()> {
    zip.start_file(name, opts).map_err(zip_err)?;
    zip.write_all(contents)?;
    Ok(())
}

fn zip_err(err: zip::result::ZipError) -> OxideError {
    OxideError::Io(std::io::Error::other(err.to_string()))
}

fn deterministic_zip_options() -> SimpleFileOptions {
    SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default())
}

#[derive(Debug)]
struct DocxImagePart {
    page: u32,
    rel_id: String,
    name: String,
    bytes: Vec<u8>,
    width_emu: i64,
    height_emu: i64,
    x_emu: i64,
    y_emu: i64,
}

#[derive(Debug, Clone)]
struct DocxHyperlinkPart {
    target: String,
    rel_id: String,
}

fn write_docx(
    engine: &ContentEngine,
    document: &Document,
    options: &DocxOptions,
) -> Result<Vec<u8>> {
    let images = if options.include_images {
        collect_docx_images(engine, document)?
    } else {
        Vec::new()
    };
    let hyperlinks = collect_docx_hyperlinks(document);

    let mut cursor = Cursor::new(Vec::new());
    let mut zip = ZipWriter::new(&mut cursor);
    let opts = deterministic_zip_options();

    zip_file(
        &mut zip,
        opts,
        "[Content_Types].xml",
        &docx_content_types(images.iter().map(|img| img.name.as_str())),
    )?;
    zip_file(&mut zip, opts, "_rels/.rels", PACKAGE_RELS_DOCX)?;
    zip_file(
        &mut zip,
        opts,
        "word/document.xml",
        &docx_document_xml(document, &images, &hyperlinks, options.layout),
    )?;
    zip_file(
        &mut zip,
        opts,
        "word/_rels/document.xml.rels",
        &docx_document_rels(&images, &hyperlinks),
    )?;
    zip_file(&mut zip, opts, "word/styles.xml", DOCX_STYLES)?;
    zip_file(&mut zip, opts, "word/numbering.xml", DOCX_NUMBERING)?;
    zip_file(&mut zip, opts, "word/settings.xml", DOCX_SETTINGS)?;
    zip_file(&mut zip, opts, "docProps/core.xml", DOCX_CORE_PROPERTIES)?;
    zip_file(&mut zip, opts, "docProps/app.xml", DOCX_APP_PROPERTIES)?;
    let mut written_media = BTreeSet::new();
    for image in &images {
        if !written_media.insert(image.name.clone()) {
            continue;
        }
        zip_bytes(
            &mut zip,
            opts,
            &format!("word/media/{}", image.name),
            &image.bytes,
        )?;
    }
    zip.finish().map_err(zip_err)?;
    Ok(cursor.into_inner())
}

fn collect_docx_images(engine: &ContentEngine, document: &Document) -> Result<Vec<DocxImagePart>> {
    let mut out = Vec::new();
    for page in &document.pages {
        let Ok(region) = PageRegion::new(0.0, 0.0, page.width.max(1.0), page.height.max(1.0))
        else {
            continue;
        };
        let Ok(images) = engine.find_page_images_in_region(page.number as usize, region) else {
            continue;
        };
        for image in images
            .into_iter()
            .filter(|img| !img.image.is_mask && !img.image.is_smask)
        {
            let Ok(bytes) = engine.extract_image_bytes(&image.image, ImageOutputFormat::Png, None)
            else {
                continue;
            };
            let width_points = (image.bbox[2] - image.bbox[0]).abs().clamp(72.0, 432.0);
            let height_points = (image.bbox[3] - image.bbox[1]).abs().clamp(72.0, 432.0);
            let x_points = image.bbox[0].max(0.0);
            let y_points = (page.height - image.bbox[3]).max(0.0);
            let digest = resource_digest(&bytes);
            let stable_suffix = &digest[..16];
            out.push(DocxImagePart {
                page: page.number,
                rel_id: format!("rIdImage{stable_suffix}"),
                name: format!("image-{stable_suffix}.png"),
                bytes,
                width_emu: points_to_emu(width_points),
                height_emu: points_to_emu(height_points),
                x_emu: points_to_emu(x_points),
                y_emu: points_to_emu(y_points),
            });
        }
    }
    Ok(out)
}

fn collect_docx_hyperlinks(document: &Document) -> Vec<DocxHyperlinkPart> {
    let mut targets = BTreeSet::new();
    for block in &document.body {
        for span in block_inline_spans(block) {
            if let Some(target) = span.link.as_ref().filter(|value| {
                let lower = value.to_ascii_lowercase();
                lower.starts_with("https://")
                    || lower.starts_with("http://")
                    || lower.starts_with("mailto:")
            }) {
                targets.insert(target.clone());
            }
        }
    }
    targets
        .into_iter()
        .map(|target| {
            let digest = resource_digest(target.as_bytes());
            DocxHyperlinkPart {
                rel_id: format!("rIdLink{}", &digest[..16]),
                target,
            }
        })
        .collect()
}

fn docx_content_types<'a>(media: impl IntoIterator<Item = &'a str>) -> String {
    let has_png = media.into_iter().any(|name| name.ends_with(".png"));
    let mut out = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
"#,
    );
    if has_png {
        out.push_str(r#"<Default Extension="png" ContentType="image/png"/>"#);
        out.push('\n');
    }
    out.push_str(
        r#"<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
<Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>
<Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/>
<Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/>
<Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/>
<Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/>
</Types>"#,
    );
    out
}

const PACKAGE_RELS_DOCX: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/>
<Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties" Target="docProps/app.xml"/>
</Relationships>"#;

fn docx_document_rels(images: &[DocxImagePart], hyperlinks: &[DocxHyperlinkPart]) -> String {
    let mut out = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="{PACKAGE_REL_NS}">"#
    );
    out.push_str(r#"<Relationship Id="rIdStyles" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>"#);
    out.push_str(r#"<Relationship Id="rIdNumbering" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml"/>"#);
    out.push_str(r#"<Relationship Id="rIdSettings" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings" Target="settings.xml"/>"#);
    let mut media = BTreeSet::new();
    for image in images {
        if !media.insert((&image.rel_id, &image.name)) {
            continue;
        }
        out.push_str(&format!(
            r#"<Relationship Id="{}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/{}"/>"#,
            xml_escape(&image.rel_id),
            xml_escape(&image.name)
        ));
    }
    for hyperlink in hyperlinks {
        out.push_str(&format!(
            r#"<Relationship Id="{}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="{}" TargetMode="External"/>"#,
            xml_escape(&hyperlink.rel_id),
            xml_escape(&hyperlink.target)
        ));
    }
    out.push_str("</Relationships>");
    out
}

fn docx_document_xml(
    document: &Document,
    images: &[DocxImagePart],
    hyperlinks: &[DocxHyperlinkPart],
    layout: DocxLayout,
) -> String {
    let mut body = String::new();
    let hyperlink_map = hyperlinks
        .iter()
        .map(|link| (link.target.as_str(), link.rel_id.as_str()))
        .collect::<BTreeMap<_, _>>();
    if document.pages.is_empty() && document.body.is_empty() {
        body.push_str(&docx_paragraph_xml(
            &[InlineSpan {
                text: "No extracted content".to_string(),
                ..Default::default()
            }],
            ParagraphRole::Normal,
            &hyperlink_map,
        ));
    }
    let mut anchor_id = 1usize;
    for (page_index, page) in document.pages.iter().enumerate() {
        let mut blocks = page_blocks(document, page.number);
        blocks.sort_by_key(|block| block.reading_order);
        for block in blocks {
            match layout {
                DocxLayout::Flowing => body.push_str(&docx_block_xml(block, &hyperlink_map)),
                DocxLayout::PageFaithful => {
                    if matches!(block.kind, BlockKind::Table { .. }) {
                        body.push_str(&docx_block_xml(block, &hyperlink_map));
                    } else {
                        body.push_str(&docx_positioned_block_xml(
                            block,
                            page,
                            anchor_id,
                            &hyperlink_map,
                        ));
                        anchor_id += 1;
                    }
                }
                DocxLayout::Hybrid => {
                    if matches!(
                        block.kind,
                        BlockKind::Title { .. }
                            | BlockKind::Heading { .. }
                            | BlockKind::Table { .. }
                            | BlockKind::List { .. }
                    ) && block.confidence >= 0.75
                    {
                        body.push_str(&docx_block_xml(block, &hyperlink_map));
                    } else {
                        body.push_str(&docx_positioned_block_xml(
                            block,
                            page,
                            anchor_id,
                            &hyperlink_map,
                        ));
                        anchor_id += 1;
                    }
                }
            }
        }
        for image in images.iter().filter(|image| image.page == page.number) {
            match layout {
                DocxLayout::Flowing => body.push_str(&docx_image_paragraph_xml(image)),
                DocxLayout::PageFaithful | DocxLayout::Hybrid => {
                    body.push_str(&docx_image_anchor_xml(image, anchor_id));
                    anchor_id += 1;
                }
            }
        }
        if page_index + 1 < document.pages.len() {
            body.push_str(&docx_section_break_xml(page, layout));
        }
    }
    if body.is_empty() {
        for block in &document.body {
            body.push_str(&docx_block_xml(block, &hyperlink_map));
        }
    }
    let final_section = document
        .pages
        .last()
        .map(|page| docx_section_properties_xml(page, layout, false))
        .unwrap_or_else(|| {
            r#"<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/></w:sectPr>"#.to_string()
        });
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="{WORD_NS}" xmlns:r="{DOCX_REL_NS}" xmlns:wp="{WP_NS}" xmlns:pic="{PIC_NS}" xmlns:a="{DRAWING_NS}" xmlns:wps="{WPS_NS}"><w:body>{body}{final_section}</w:body></w:document>"#
    )
}

fn docx_section_break_xml(page: &crate::parse::Page, layout: DocxLayout) -> String {
    format!(
        "<w:p><w:pPr>{}</w:pPr></w:p>",
        docx_section_properties_xml(page, layout, true)
    )
}

fn docx_section_properties_xml(
    page: &crate::parse::Page,
    layout: DocxLayout,
    next_page: bool,
) -> String {
    let width = (page.width.max(1.0) * 20.0).round() as i64;
    let height = (page.height.max(1.0) * 20.0).round() as i64;
    let orient = if width > height {
        r#" w:orient="landscape""#
    } else {
        ""
    };
    let margin = match layout {
        DocxLayout::Flowing => 720,
        DocxLayout::Hybrid => 360,
        DocxLayout::PageFaithful => 0,
    };
    let break_type = if next_page {
        r#"<w:type w:val="nextPage"/>"#
    } else {
        ""
    };
    format!(
        r#"<w:sectPr>{break_type}<w:pgSz w:w="{width}" w:h="{height}"{orient}/><w:pgMar w:top="{margin}" w:right="{margin}" w:bottom="{margin}" w:left="{margin}" w:header="0" w:footer="0" w:gutter="0"/><w:cols w:num="1"/><w:docGrid w:linePitch="360"/></w:sectPr>"#
    )
}

fn docx_block_xml(block: &Block, hyperlinks: &BTreeMap<&str, &str>) -> String {
    match &block.kind {
        BlockKind::Title { text } => {
            docx_paragraph_xml(&text.spans, ParagraphRole::Title, hyperlinks)
        }
        BlockKind::Heading { level, text } => docx_paragraph_xml(
            &text.spans,
            ParagraphRole::Heading((*level).clamp(1, 3)),
            hyperlinks,
        ),
        BlockKind::Paragraph { text }
        | BlockKind::Caption { text, .. }
        | BlockKind::Header { text }
        | BlockKind::Footer { text }
        | BlockKind::PageNumber { text }
        | BlockKind::Text { text } => {
            docx_paragraph_xml(&text.spans, ParagraphRole::Normal, hyperlinks)
        }
        BlockKind::List { ordered, items } => {
            let mut out = String::new();
            let num_id = if *ordered { 2 } else { 1 };
            for item in items {
                out.push_str(&docx_list_paragraph_xml(
                    &item.text.spans,
                    num_id,
                    hyperlinks,
                ));
            }
            out
        }
        BlockKind::Table { table, .. } => docx_table_xml(table, hyperlinks),
        BlockKind::Figure { alt, .. } => alt
            .as_ref()
            .filter(|text| !text.trim().is_empty())
            .map(|text| {
                docx_paragraph_xml(
                    &[InlineSpan {
                        text: text.clone(),
                        italic: true,
                        ..Default::default()
                    }],
                    ParagraphRole::Normal,
                    hyperlinks,
                )
            })
            .unwrap_or_default(),
    }
}

fn docx_paragraph_xml(
    spans: &[InlineSpan],
    role: ParagraphRole,
    hyperlinks: &BTreeMap<&str, &str>,
) -> String {
    if spans.iter().all(|span| span.text.trim().is_empty()) {
        return String::new();
    }
    let style = match role {
        ParagraphRole::Normal => r#"<w:widowControl/><w:keepLines/>"#.to_string(),
        ParagraphRole::Title => {
            r#"<w:pStyle w:val="Title"/><w:keepNext/><w:keepLines/><w:widowControl/>"#.to_string()
        }
        ParagraphRole::Heading(level) => format!(
            r#"<w:pStyle w:val="Heading{level}"/><w:keepNext/><w:keepLines/><w:widowControl/>"#
        ),
    };
    format!(
        "<w:p><w:pPr>{style}</w:pPr>{}</w:p>",
        spans
            .iter()
            .map(|span| docx_run_xml(span, hyperlinks))
            .collect::<String>()
    )
}

fn docx_list_paragraph_xml(
    spans: &[InlineSpan],
    num_id: u8,
    hyperlinks: &BTreeMap<&str, &str>,
) -> String {
    if spans.iter().all(|span| span.text.trim().is_empty()) {
        return String::new();
    }
    format!(
        r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="{num_id}"/></w:numPr></w:pPr>{}</w:p>"#,
        spans
            .iter()
            .map(|span| docx_run_xml(span, hyperlinks))
            .collect::<String>()
    )
}

fn docx_run_xml(span: &InlineSpan, hyperlinks: &BTreeMap<&str, &str>) -> String {
    if span.text.is_empty() {
        return String::new();
    }
    let mut props = String::new();
    if span.bold {
        props.push_str("<w:b/>");
    }
    if span.italic {
        props.push_str("<w:i/>");
    }
    if span.link.is_some() {
        props.push_str(r#"<w:color w:val="0563C1"/><w:u w:val="single"/>"#);
    }
    let props = if props.is_empty() {
        String::new()
    } else {
        format!("<w:rPr>{props}</w:rPr>")
    };
    let preserve = if span.text.starts_with(' ') || span.text.ends_with(' ') {
        r#" xml:space="preserve""#
    } else {
        ""
    };
    let run = format!(
        r#"<w:r>{props}<w:t{preserve}>{}</w:t></w:r>"#,
        xml_escape(&span.text)
    );
    if let Some(rel_id) = span
        .link
        .as_deref()
        .and_then(|target| hyperlinks.get(target).copied())
    {
        format!(
            r#"<w:hyperlink r:id="{}" w:history="1">{run}</w:hyperlink>"#,
            xml_escape(rel_id)
        )
    } else {
        run
    }
}

fn docx_positioned_block_xml(
    block: &Block,
    page: &crate::parse::Page,
    anchor_id: usize,
    hyperlinks: &BTreeMap<&str, &str>,
) -> String {
    let spans = block_inline_spans(block);
    if spans.is_empty() {
        return String::new();
    }
    if spans.iter().all(|span| span.text.trim().is_empty()) {
        return String::new();
    }
    let x = block.bbox[0].max(0.0);
    let y = (page.height - block.bbox[3]).max(0.0);
    let w = (block.bbox[2] - block.bbox[0])
        .abs()
        .clamp(36.0, page.width.max(36.0));
    let h = (block.bbox[3] - block.bbox[1])
        .abs()
        .clamp(14.0, page.height.max(14.0));
    let role = match &block.kind {
        BlockKind::Title { .. } => ParagraphRole::Title,
        BlockKind::Heading { level, .. } => ParagraphRole::Heading((*level).clamp(1, 3)),
        _ => ParagraphRole::Normal,
    };
    let paragraph = docx_paragraph_xml(&spans, role, hyperlinks);
    docx_textbox_anchor_xml(
        anchor_id,
        points_to_emu(x),
        points_to_emu(y),
        points_to_emu(w),
        points_to_emu(h),
        &paragraph,
    )
}

fn docx_textbox_anchor_xml(
    anchor_id: usize,
    x_emu: i64,
    y_emu: i64,
    width_emu: i64,
    height_emu: i64,
    content_xml: &str,
) -> String {
    format!(
        r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0"/></w:pPr><w:r><w:drawing><wp:anchor distT="0" distB="0" distL="0" distR="0" simplePos="0" relativeHeight="{anchor_id}" behindDoc="0" locked="0" layoutInCell="1" allowOverlap="1"><wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="page"><wp:posOffset>{x_emu}</wp:posOffset></wp:positionH><wp:positionV relativeFrom="page"><wp:posOffset>{y_emu}</wp:posOffset></wp:positionV><wp:extent cx="{width_emu}" cy="{height_emu}"/><wp:effectExtent l="0" t="0" r="0" b="0"/><wp:wrapNone/><wp:docPr id="{anchor_id}" name="OxideBlock{anchor_id}" descr="PDF positioned text block"/><wp:cNvGraphicFramePr><a:graphicFrameLocks noChangeAspect="1"/></wp:cNvGraphicFramePr><a:graphic><a:graphicData uri="{WPS_NS}"><wps:wsp><wps:cNvSpPr txBox="1"/><wps:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{width_emu}" cy="{height_emu}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:noFill/><a:ln><a:noFill/></a:ln></wps:spPr><wps:txbx><w:txbxContent>{content_xml}</w:txbxContent></wps:txbx><wps:bodyPr lIns="0" tIns="0" rIns="0" bIns="0" wrap="none"/></wps:wsp></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r></w:p>"#
    )
}

fn block_inline_spans(block: &Block) -> Vec<InlineSpan> {
    match &block.kind {
        BlockKind::Title { text }
        | BlockKind::Heading { text, .. }
        | BlockKind::Paragraph { text }
        | BlockKind::Caption { text, .. }
        | BlockKind::Header { text }
        | BlockKind::Footer { text }
        | BlockKind::PageNumber { text }
        | BlockKind::Text { text } => text.spans.clone(),
        BlockKind::List { items, .. } => items
            .iter()
            .flat_map(|item| item.text.spans.clone())
            .collect(),
        BlockKind::Figure { alt, .. } => alt
            .as_ref()
            .map(|text| {
                vec![InlineSpan {
                    text: text.clone(),
                    italic: true,
                    ..Default::default()
                }]
            })
            .unwrap_or_default(),
        BlockKind::Table { .. } => Vec::new(),
    }
}

#[derive(Debug, Clone)]
struct DocxGridCell {
    text: String,
    is_header: bool,
    rowspan: usize,
    colspan: usize,
}

fn docx_table_xml(table: &Table, hyperlinks: &BTreeMap<&str, &str>) -> String {
    let rows = table.num_rows().max(1);
    let cols = table.num_cols().max(1);
    let mut origins: HashMap<(usize, usize), DocxGridCell> = HashMap::new();
    let mut continuations: HashMap<(usize, usize), usize> = HashMap::new();

    if table.cells.is_empty() {
        for (row_idx, row) in table.rows.iter().enumerate() {
            for (col_idx, text) in row.iter().enumerate() {
                origins.insert(
                    (row_idx, col_idx),
                    DocxGridCell {
                        text: text.clone(),
                        is_header: row_idx == 0,
                        rowspan: 1,
                        colspan: 1,
                    },
                );
            }
        }
    } else {
        for cell in &table.cells {
            let rowspan = cell.rowspan.max(1);
            let colspan = cell.colspan.max(1);
            origins.insert(
                (cell.row, cell.col),
                DocxGridCell {
                    text: cell.text.clone(),
                    is_header: cell.is_header,
                    rowspan,
                    colspan,
                },
            );
            for row in cell.row + 1..cell.row + rowspan {
                continuations.insert((row, cell.col), colspan);
            }
        }
    }

    let grid_width = (9000 / cols.max(1)).max(720);
    let mut out = String::from(
        r#"<w:tbl><w:tblPr><w:tblStyle w:val="TableGrid"/><w:tblW w:w="0" w:type="auto"/><w:tblLook w:val="04A0"/></w:tblPr><w:tblGrid>"#,
    );
    for _ in 0..cols {
        out.push_str(&format!(r#"<w:gridCol w:w="{grid_width}"/>"#));
    }
    out.push_str("</w:tblGrid>");
    for row in 0..rows {
        if row == 0 {
            out.push_str(r#"<w:tr><w:trPr><w:tblHeader/><w:cantSplit/></w:trPr>"#);
        } else {
            out.push_str(r#"<w:tr><w:trPr><w:cantSplit/></w:trPr>"#);
        }
        let mut col = 0usize;
        while col < cols {
            if let Some(cell) = origins.get(&(row, col)) {
                out.push_str(&docx_table_cell_xml(
                    &cell.text,
                    cell.is_header,
                    cell.rowspan,
                    cell.colspan,
                    false,
                    hyperlinks,
                ));
                col += cell.colspan.max(1);
            } else if let Some(colspan) = continuations.get(&(row, col)) {
                out.push_str(&docx_table_cell_xml(
                    "", false, 1, *colspan, true, hyperlinks,
                ));
                col += (*colspan).max(1);
            } else {
                out.push_str(&docx_table_cell_xml("", false, 1, 1, false, hyperlinks));
                col += 1;
            }
        }
        out.push_str("</w:tr>");
    }
    out.push_str("</w:tbl>");
    out
}

fn docx_table_cell_xml(
    text: &str,
    is_header: bool,
    rowspan: usize,
    colspan: usize,
    vmerge_continue: bool,
    hyperlinks: &BTreeMap<&str, &str>,
) -> String {
    let mut props = String::new();
    if colspan > 1 {
        props.push_str(&format!(r#"<w:gridSpan w:val="{colspan}"/>"#));
    }
    if rowspan > 1 {
        props.push_str(r#"<w:vMerge w:val="restart"/>"#);
    }
    if vmerge_continue {
        props.push_str("<w:vMerge/>");
    }
    if is_header {
        props.push_str(r#"<w:shd w:fill="E5E7EB"/>"#);
    }
    let spans = [InlineSpan {
        text: text.to_string(),
        bold: is_header,
        ..Default::default()
    }];
    format!(
        "<w:tc><w:tcPr>{props}</w:tcPr>{}</w:tc>",
        docx_paragraph_xml(&spans, ParagraphRole::Normal, hyperlinks)
    )
}

fn docx_image_paragraph_xml(image: &DocxImagePart) -> String {
    format!(
        r#"<w:p><w:r><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0"><wp:extent cx="{}" cy="{}"/><wp:docPr id="1" name="{}"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="0" name="{}"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="{}"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{}" cy="{}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#,
        image.width_emu,
        image.height_emu,
        xml_escape(&image.name),
        xml_escape(&image.name),
        xml_escape(&image.rel_id),
        image.width_emu,
        image.height_emu
    )
}

fn docx_image_anchor_xml(image: &DocxImagePart, anchor_id: usize) -> String {
    format!(
        r#"<w:p><w:r><w:drawing><wp:anchor distT="0" distB="0" distL="0" distR="0" simplePos="0" relativeHeight="0" behindDoc="0" locked="0" layoutInCell="1" allowOverlap="1"><wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="page"><wp:posOffset>{}</wp:posOffset></wp:positionH><wp:positionV relativeFrom="page"><wp:posOffset>{}</wp:posOffset></wp:positionV><wp:extent cx="{}" cy="{}"/><wp:wrapNone/><wp:docPr id="{}" name="{}"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="{}" name="{}"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="{}"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{}" cy="{}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r></w:p>"#,
        image.x_emu,
        image.y_emu,
        image.width_emu,
        image.height_emu,
        anchor_id,
        xml_escape(&image.name),
        anchor_id,
        xml_escape(&image.name),
        xml_escape(&image.rel_id),
        image.width_emu,
        image.height_emu
    )
}

const DOCX_STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/><w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri"/><w:sz w:val="22"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Title"><w:name w:val="Title"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/><w:rPr><w:b/><w:sz w:val="44"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/><w:pPr><w:keepNext/></w:pPr><w:rPr><w:b/><w:sz w:val="32"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/><w:pPr><w:keepNext/></w:pPr><w:rPr><w:b/><w:sz w:val="28"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading3"><w:name w:val="heading 3"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/><w:pPr><w:keepNext/></w:pPr><w:rPr><w:b/><w:i/><w:sz w:val="24"/></w:rPr></w:style>
<w:style w:type="table" w:styleId="TableGrid"><w:name w:val="Table Grid"/><w:tblPr><w:tblBorders><w:top w:val="single" w:sz="4" w:space="0" w:color="A0A0A0"/><w:left w:val="single" w:sz="4" w:space="0" w:color="A0A0A0"/><w:bottom w:val="single" w:sz="4" w:space="0" w:color="A0A0A0"/><w:right w:val="single" w:sz="4" w:space="0" w:color="A0A0A0"/><w:insideH w:val="single" w:sz="4" w:space="0" w:color="A0A0A0"/><w:insideV w:val="single" w:sz="4" w:space="0" w:color="A0A0A0"/></w:tblBorders></w:tblPr></w:style>
</w:styles>"#;

const DOCX_NUMBERING: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="bullet"/><w:lvlText w:val="•"/><w:lvlJc w:val="left"/></w:lvl></w:abstractNum>
<w:abstractNum w:abstractNumId="2"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/><w:lvlJc w:val="left"/></w:lvl></w:abstractNum>
<w:num w:numId="1"><w:abstractNumId w:val="1"/></w:num>
<w:num w:numId="2"><w:abstractNumId w:val="2"/></w:num>
</w:numbering>"#;

const DOCX_SETTINGS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:zoom w:percent="100"/><w:defaultTabStop w:val="720"/><w:evenAndOddHeaders/>
<w:compat><w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="15"/></w:compat>
</w:settings>"#;

const DOCX_CORE_PROPERTIES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:dcmitype="http://purl.org/dc/dcmitype/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
<dc:title>Oxide PDF conversion</dc:title><dc:creator>Oxide PDF SDK</dc:creator><cp:lastModifiedBy>Oxide PDF SDK</cp:lastModifiedBy><dcterms:created xsi:type="dcterms:W3CDTF">1980-01-01T00:00:00Z</dcterms:created><dcterms:modified xsi:type="dcterms:W3CDTF">1980-01-01T00:00:00Z</dcterms:modified><cp:revision>1</cp:revision>
</cp:coreProperties>"#;

const DOCX_APP_PROPERTIES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"><Application>Oxide PDF SDK</Application><AppVersion>1.0</AppVersion></Properties>"#;

fn xlsx_content_types(sheet_count: usize) -> String {
    let mut out = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
<Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>
"#,
    );
    for idx in 1..=sheet_count {
        out.push_str(&format!(
            r#"<Override PartName="/xl/worksheets/sheet{idx}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#
        ));
        out.push('\n');
    }
    out.push_str("</Types>");
    out
}

const PACKAGE_RELS_XLSX: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#;

fn xlsx_workbook(sheets: &[XlsxSheet]) -> String {
    let mut out = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="{XLSX_MAIN_NS}" xmlns:r="{REL_NS}"><sheets>"#
    );
    for (idx, sheet) in sheets.iter().enumerate() {
        out.push_str(&format!(
            r#"<sheet name="{}" sheetId="{}" r:id="rId{}"/>"#,
            xml_escape(&sheet.name),
            idx + 1,
            idx + 1
        ));
    }
    out.push_str("</sheets></workbook>");
    out
}

fn xlsx_workbook_rels(sheet_count: usize) -> String {
    let mut out = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="{PACKAGE_REL_NS}">"#
    );
    for idx in 1..=sheet_count {
        out.push_str(&format!(
            r#"<Relationship Id="rId{idx}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet{idx}.xml"/>"#
        ));
    }
    out.push_str(r#"<Relationship Id="rIdStyles" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>"#);
    out.push_str("</Relationships>");
    out
}

const XLSX_STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
<fonts count="2"><font><sz val="11"/><name val="Calibri"/></font><font><b/><sz val="11"/><name val="Calibri"/></font></fonts>
<fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills>
<borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders>
<cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>
<cellXfs count="2"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="0" fontId="1" fillId="0" borderId="0" xfId="0" applyFont="1"/></cellXfs>
<cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles>
</styleSheet>"#;

fn xlsx_sheet_xml(sheet: &XlsxSheet) -> String {
    let max_row = sheet.rows.keys().copied().max().unwrap_or(1);
    let max_col = sheet
        .rows
        .values()
        .flat_map(|cells| cells.iter().map(|cell| cell.col))
        .max()
        .unwrap_or(1);
    let mut out = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="{XLSX_MAIN_NS}" xmlns:r="{REL_NS}"><dimension ref="A1:{}"/>"#,
        cell_ref(max_row, max_col)
    );
    if !sheet.col_widths.is_empty() {
        out.push_str("<cols>");
        for (col, width) in &sheet.col_widths {
            out.push_str(&format!(
                r#"<col min="{col}" max="{col}" width="{:.2}" customWidth="1"/>"#,
                width.min(80.0)
            ));
        }
        out.push_str("</cols>");
    }
    out.push_str("<sheetData>");
    for (row, cells) in &sheet.rows {
        out.push_str(&format!(r#"<row r="{row}">"#));
        let mut sorted = cells.clone();
        sorted.sort_by_key(|cell| cell.col);
        for cell in sorted {
            out.push_str(&xlsx_cell_xml(*row, &cell));
        }
        out.push_str("</row>");
    }
    out.push_str("</sheetData>");
    if !sheet.merges.is_empty() {
        out.push_str(&format!(r#"<mergeCells count="{}">"#, sheet.merges.len()));
        for merge in &sheet.merges {
            out.push_str(&format!(r#"<mergeCell ref="{}"/>"#, xml_escape(merge)));
        }
        out.push_str("</mergeCells>");
    }
    out.push_str("</worksheet>");
    out
}

fn xlsx_cell_xml(row: usize, cell: &XlsxCell) -> String {
    let style = if cell.style == 0 {
        String::new()
    } else {
        format!(r#" s="{}""#, cell.style)
    };
    let reference = cell_ref(row, cell.col);
    if let Some(number) = parse_excel_number(&cell.text) {
        format!(r#"<c r="{reference}"{style}><v>{number}</v></c>"#)
    } else {
        format!(
            r#"<c r="{reference}" t="inlineStr"{style}><is><t>{}</t></is></c>"#,
            xml_escape(&cell.text)
        )
    }
}

fn parse_excel_number(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.chars().any(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let mut s = trimmed.replace(',', "");
    let percent = s.ends_with('%');
    if percent {
        s.pop();
    }
    if let Some(rest) = s.strip_prefix('$') {
        s = rest.to_string();
    }
    if s.starts_with('0') && s.len() > 1 && !s.starts_with("0.") {
        return None;
    }
    let value: f64 = s.parse().ok()?;
    if !value.is_finite() {
        return None;
    }
    let value = if percent { value / 100.0 } else { value };
    Some(format_number(value))
}

fn format_number(value: f64) -> String {
    let mut s = format!("{value:.12}");
    while s.contains('.') && s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    s
}

fn write_pptx(
    engine: &ContentEngine,
    document: &Document,
    options: &PptxOptions,
) -> Result<Vec<u8>> {
    let mut cursor = Cursor::new(Vec::new());
    let mut zip = ZipWriter::new(&mut cursor);
    let opts = deterministic_zip_options();

    let slide_count = document.pages.len().max(1);
    let (slide_cx, slide_cy) = ppt_slide_size(document, engine)?;
    zip_file(
        &mut zip,
        opts,
        "[Content_Types].xml",
        &pptx_content_types(slide_count),
    )?;
    zip_file(&mut zip, opts, "_rels/.rels", PACKAGE_RELS_PPTX)?;
    zip_file(
        &mut zip,
        opts,
        "ppt/presentation.xml",
        &pptx_presentation(slide_count, slide_cx, slide_cy),
    )?;
    zip_file(
        &mut zip,
        opts,
        "ppt/_rels/presentation.xml.rels",
        &pptx_presentation_rels(slide_count),
    )?;
    zip_file(&mut zip, opts, "ppt/theme/theme1.xml", PPTX_THEME)?;
    zip_file(
        &mut zip,
        opts,
        "ppt/slideMasters/slideMaster1.xml",
        PPTX_MASTER,
    )?;
    zip_file(
        &mut zip,
        opts,
        "ppt/slideMasters/_rels/slideMaster1.xml.rels",
        PPTX_MASTER_RELS,
    )?;
    zip_file(
        &mut zip,
        opts,
        "ppt/slideLayouts/slideLayout1.xml",
        PPTX_LAYOUT,
    )?;
    zip_file(
        &mut zip,
        opts,
        "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
        PPTX_LAYOUT_RELS,
    )?;

    let mut media_index = 1usize;
    for (idx, page) in document.pages.iter().enumerate() {
        let page_number = page.number as usize;
        let (page_w, page_h) = page_dimensions(document, engine, page.number)?;
        let mut rels = vec![(
            "rId1".to_string(),
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout"
                .to_string(),
            "../slideLayouts/slideLayout1.xml".to_string(),
        )];
        let mut media = Vec::new();
        let slide_xml = pptx_slide_xml(
            PptxSlideContext {
                engine,
                document,
                page_number: page.number,
                page_size: (page_w, page_h),
                slide_size: (slide_cx, slide_cy),
                options,
            },
            &mut media_index,
            &mut rels,
            &mut media,
        );
        zip_file(
            &mut zip,
            opts,
            &format!("ppt/slides/slide{}.xml", idx + 1),
            &slide_xml,
        )?;
        zip_file(
            &mut zip,
            opts,
            &format!("ppt/slides/_rels/slide{}.xml.rels", idx + 1),
            &pptx_slide_rels(&rels),
        )?;
        for (name, bytes) in media {
            zip_bytes(&mut zip, opts, &name, &bytes)?;
        }
        let _ = page_number;
    }
    if document.pages.is_empty() {
        zip_file(
            &mut zip,
            opts,
            "ppt/slides/slide1.xml",
            &empty_pptx_slide_xml(slide_cx, slide_cy),
        )?;
        zip_file(
            &mut zip,
            opts,
            "ppt/slides/_rels/slide1.xml.rels",
            &pptx_slide_rels(&[(
                "rId1".to_string(),
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout"
                    .to_string(),
                "../slideLayouts/slideLayout1.xml".to_string(),
            )]),
        )?;
    }

    zip.finish().map_err(zip_err)?;
    Ok(cursor.into_inner())
}

const PACKAGE_RELS_PPTX: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/>
</Relationships>"#;

fn pptx_content_types(slide_count: usize) -> String {
    let mut out = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Default Extension="png" ContentType="image/png"/>
<Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>
<Override PartName="/ppt/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/>
<Override PartName="/ppt/slideMasters/slideMaster1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml"/>
<Override PartName="/ppt/slideLayouts/slideLayout1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"/>
"#,
    );
    for idx in 1..=slide_count {
        out.push_str(&format!(
            r#"<Override PartName="/ppt/slides/slide{idx}.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>"#
        ));
        out.push('\n');
    }
    out.push_str("</Types>");
    out
}

fn pptx_presentation(slide_count: usize, slide_cx: i64, slide_cy: i64) -> String {
    let mut out = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation xmlns:a="{DRAWING_NS}" xmlns:r="{REL_NS}" xmlns:p="{PPT_NS}">
<p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst><p:sldIdLst>"#
    );
    for idx in 1..=slide_count {
        out.push_str(&format!(
            r#"<p:sldId id="{}" r:id="rId{}"/>"#,
            255 + idx,
            idx + 1
        ));
    }
    out.push_str(&format!(
        r#"</p:sldIdLst><p:sldSz cx="{slide_cx}" cy="{slide_cy}" type="custom"/><p:notesSz cx="6858000" cy="9144000"/></p:presentation>"#
    ));
    out
}

fn pptx_presentation_rels(slide_count: usize) -> String {
    let mut out = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="{PACKAGE_REL_NS}">"#
    );
    out.push_str(r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="slideMasters/slideMaster1.xml"/>"#);
    for idx in 1..=slide_count {
        out.push_str(&format!(
            r#"<Relationship Id="rId{}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide{idx}.xml"/>"#,
            idx + 1
        ));
    }
    out.push_str("</Relationships>");
    out
}

const PPTX_THEME: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Oxide"><a:themeElements><a:clrScheme name="Oxide"><a:dk1><a:srgbClr val="111111"/></a:dk1><a:lt1><a:srgbClr val="FFFFFF"/></a:lt1><a:dk2><a:srgbClr val="1F2937"/></a:dk2><a:lt2><a:srgbClr val="F8FAFC"/></a:lt2><a:accent1><a:srgbClr val="2563EB"/></a:accent1><a:accent2><a:srgbClr val="16A34A"/></a:accent2><a:accent3><a:srgbClr val="DC2626"/></a:accent3><a:accent4><a:srgbClr val="9333EA"/></a:accent4><a:accent5><a:srgbClr val="EA580C"/></a:accent5><a:accent6><a:srgbClr val="0891B2"/></a:accent6><a:hlink><a:srgbClr val="2563EB"/></a:hlink><a:folHlink><a:srgbClr val="9333EA"/></a:folHlink></a:clrScheme><a:fontScheme name="Oxide"><a:majorFont><a:latin typeface="Arial"/></a:majorFont><a:minorFont><a:latin typeface="Arial"/></a:minorFont></a:fontScheme><a:fmtScheme name="Oxide"><a:fillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:fillStyleLst><a:lnStyleLst><a:ln w="9525"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln></a:lnStyleLst><a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst><a:bgFillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:bgFillStyleLst></a:fmtScheme></a:themeElements></a:theme>"#;

const PPTX_MASTER: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sldMaster xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:bg><p:bgPr><a:solidFill><a:srgbClr val="FFFFFF"/></a:solidFill></p:bgPr></p:bg><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/></p:spTree></p:cSld><p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/><p:sldLayoutIdLst><p:sldLayoutId id="2147483649" r:id="rId1"/></p:sldLayoutIdLst><p:txStyles><p:titleStyle/><p:bodyStyle/><p:otherStyle/></p:txStyles></p:sldMaster>"#;

const PPTX_MASTER_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="../theme/theme1.xml"/></Relationships>"#;

const PPTX_LAYOUT: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sldLayout xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" type="blank" preserve="1"><p:cSld name="Blank"><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/></p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>"#;

const PPTX_LAYOUT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="../slideMasters/slideMaster1.xml"/></Relationships>"#;

struct PptxSlideContext<'a> {
    engine: &'a ContentEngine,
    document: &'a Document,
    page_number: u32,
    page_size: (f64, f64),
    slide_size: (i64, i64),
    options: &'a PptxOptions,
}

fn pptx_slide_xml(
    ctx: PptxSlideContext<'_>,
    media_index: &mut usize,
    rels: &mut Vec<(String, String, String)>,
    media: &mut Vec<(String, Vec<u8>)>,
) -> String {
    let mut shapes = String::new();
    let mut shape_id = 2usize;
    let mut blocks = page_blocks(ctx.document, ctx.page_number);
    blocks.sort_by_key(|block| block.reading_order);
    for block in blocks {
        match &block.kind {
            BlockKind::Table { table, .. } => {
                shapes.push_str(&pptx_table_shape(
                    shape_id,
                    table,
                    block_bbox(block, Some(table.bbox)),
                    ctx.page_size,
                    ctx.slide_size,
                ));
                shape_id += 1;
            }
            _ => {
                if let Some(text) = block_text(block) {
                    shapes.push_str(&pptx_text_shape(
                        shape_id,
                        &text,
                        block,
                        block.bbox,
                        ctx.page_size,
                        ctx.slide_size,
                    ));
                    shape_id += 1;
                }
            }
        }
    }

    if ctx.options.include_images {
        if let Ok(region) = PageRegion::new(0.0, 0.0, ctx.page_size.0, ctx.page_size.1) {
            if let Ok(images) = ctx
                .engine
                .find_page_images_in_region(ctx.page_number as usize, region)
            {
                for image in images
                    .into_iter()
                    .filter(|img| !img.image.is_mask && !img.image.is_smask)
                {
                    if let Ok(bytes) =
                        ctx.engine
                            .extract_image_bytes(&image.image, ImageOutputFormat::Png, None)
                    {
                        let rel_id = format!("rId{}", rels.len() + 1);
                        let media_name = format!("ppt/media/image{}.png", *media_index);
                        *media_index += 1;
                        rels.push((
                            rel_id.clone(),
                            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image"
                                .to_string(),
                            format!("../media/{}", media_name.rsplit('/').next().unwrap_or("image.png")),
                        ));
                        shapes.push_str(&pptx_picture_shape(
                            shape_id,
                            &rel_id,
                            image.bbox,
                            ctx.page_size,
                            ctx.slide_size,
                        ));
                        shape_id += 1;
                        media.push((media_name, bytes));
                    }
                }
            }
        }
    }

    if shapes.is_empty() {
        shapes.push_str(&pptx_text_shape_raw(
            2,
            "No extracted content",
            (914400, 914400, 5486400, 914400),
            false,
            1800,
        ));
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:a="{DRAWING_NS}" xmlns:r="{REL_NS}" xmlns:p="{PPT_NS}"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>{shapes}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>"#
    )
}

fn empty_pptx_slide_xml(slide_cx: i64, slide_cy: i64) -> String {
    let _ = (slide_cx, slide_cy);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:a="{DRAWING_NS}" xmlns:r="{REL_NS}" xmlns:p="{PPT_NS}"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>{}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>"#,
        pptx_text_shape_raw(
            2,
            "No extracted content",
            (914400, 914400, 5486400, 914400),
            false,
            1800
        )
    )
}

fn pptx_slide_rels(rels: &[(String, String, String)]) -> String {
    let mut out = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="{PACKAGE_REL_NS}">"#
    );
    for (id, ty, target) in rels {
        out.push_str(&format!(
            r#"<Relationship Id="{}" Type="{}" Target="{}"/>"#,
            xml_escape(id),
            xml_escape(ty),
            xml_escape(target)
        ));
    }
    out.push_str("</Relationships>");
    out
}

fn ppt_slide_size(document: &Document, engine: &ContentEngine) -> Result<(i64, i64)> {
    let mut size = None;
    for page in &document.pages {
        let dims = page_dimensions(document, engine, page.number)?;
        if dims.0 > 0.0 && dims.1 > 0.0 {
            size = Some(dims);
            break;
        }
    }
    let (w, h) = size.unwrap_or((612.0, 792.0));
    Ok((points_to_emu(w), points_to_emu(h)))
}

fn page_dimensions(
    document: &Document,
    engine: &ContentEngine,
    page_number: u32,
) -> Result<(f64, f64)> {
    if let Some(page) = document
        .pages
        .iter()
        .find(|page| page.number == page_number)
    {
        if page.width.is_finite()
            && page.height.is_finite()
            && page.width > 0.0
            && page.height > 0.0
        {
            return Ok((page.width, page.height));
        }
    }
    engine.page_dimensions(page_number as usize)
}

fn page_blocks(document: &Document, page_number: u32) -> Vec<&Block> {
    document
        .body
        .iter()
        .filter(|block| block.page == page_number)
        .collect()
}

fn block_text(block: &Block) -> Option<String> {
    let text = match &block.kind {
        BlockKind::Title { text }
        | BlockKind::Heading { text, .. }
        | BlockKind::Paragraph { text }
        | BlockKind::Caption { text, .. }
        | BlockKind::Header { text }
        | BlockKind::Footer { text }
        | BlockKind::PageNumber { text }
        | BlockKind::Text { text } => inline_plain(text),
        BlockKind::List { ordered, items } => items
            .iter()
            .enumerate()
            .map(|(idx, item)| {
                let marker = item.marker.clone().unwrap_or_else(|| {
                    if *ordered {
                        format!("{}.", idx + 1)
                    } else {
                        "-".to_string()
                    }
                });
                format!("{marker} {}", inline_plain(&item.text))
            })
            .collect::<Vec<_>>()
            .join("\n"),
        BlockKind::Figure { alt, .. } => alt.clone().unwrap_or_default(),
        BlockKind::Table { .. } => String::new(),
    };
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn inline_plain(text: &InlineText) -> String {
    text.to_plain()
}

fn block_bbox(block: &Block, fallback: Option<[f64; 4]>) -> [f64; 4] {
    if bbox_valid(block.bbox) {
        block.bbox
    } else {
        fallback.unwrap_or([36.0, 36.0, 540.0, 120.0])
    }
}

fn bbox_valid(bbox: [f64; 4]) -> bool {
    bbox.iter().all(|v| v.is_finite()) && bbox[2] > bbox[0] && bbox[3] > bbox[1]
}

fn pptx_text_shape(
    id: usize,
    text: &str,
    block: &Block,
    bbox: [f64; 4],
    page_size: (f64, f64),
    slide_size: (i64, i64),
) -> String {
    let bold = matches!(
        block.kind,
        BlockKind::Title { .. } | BlockKind::Heading { .. }
    );
    let font_size = match block.kind {
        BlockKind::Title { .. } => 2600,
        BlockKind::Heading { .. } => 2100,
        _ => 1400,
    };
    pptx_text_shape_raw(
        id,
        text,
        map_bbox(bbox, page_size, slide_size),
        bold,
        font_size,
    )
}

fn pptx_text_shape_raw(
    id: usize,
    text: &str,
    geom: (i64, i64, i64, i64),
    bold: bool,
    font_size: i32,
) -> String {
    let (x, y, cx, cy) = geom;
    let bold_attr = if bold { r#" b="1""# } else { "" };
    let paragraphs = text
        .lines()
        .map(|line| {
            format!(
                r#"<a:p><a:r><a:rPr lang="en-US" sz="{font_size}"{bold_attr}/><a:t>{}</a:t></a:r></a:p>"#,
                xml_escape(line)
            )
        })
        .collect::<String>();
    format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="{id}" name="Text {id}"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:noFill/><a:ln><a:noFill/></a:ln></p:spPr><p:txBody><a:bodyPr wrap="square"/><a:lstStyle/>{paragraphs}</p:txBody></p:sp>"#
    )
}

fn pptx_picture_shape(
    id: usize,
    rel_id: &str,
    bbox: [f64; 4],
    page_size: (f64, f64),
    slide_size: (i64, i64),
) -> String {
    let (x, y, cx, cy) = map_bbox(bbox, page_size, slide_size);
    format!(
        r#"<p:pic><p:nvPicPr><p:cNvPr id="{id}" name="Picture {id}"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed="{}"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr><a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr></p:pic>"#,
        xml_escape(rel_id)
    )
}

fn pptx_table_shape(
    id: usize,
    table: &Table,
    bbox: [f64; 4],
    page_size: (f64, f64),
    slide_size: (i64, i64),
) -> String {
    let (x, y, cx, cy) = map_bbox(bbox, page_size, slide_size);
    let rows = table.num_rows().max(1);
    let cols = table.num_cols().max(1);
    let col_width = (cx / cols as i64).max(1);
    let row_height = (cy / rows as i64).max(1);
    let mut grid = vec![vec![None::<(&str, bool)>; cols]; rows];
    if table.cells.is_empty() {
        for (r, values) in table.rows.iter().enumerate() {
            for (c, text) in values.iter().enumerate() {
                if r < rows && c < cols {
                    grid[r][c] = Some((text.as_str(), r == 0));
                }
            }
        }
    } else {
        for cell in &table.cells {
            if cell.row < rows && cell.col < cols {
                grid[cell.row][cell.col] = Some((cell.text.as_str(), cell.is_header));
            }
        }
    }
    let mut tbl = String::from("<a:tbl><a:tblPr firstRow=\"1\" bandRow=\"1\"/><a:tblGrid>");
    for _ in 0..cols {
        tbl.push_str(&format!(r#"<a:gridCol w="{col_width}"/>"#));
    }
    tbl.push_str("</a:tblGrid>");
    for row in grid {
        tbl.push_str(&format!(r#"<a:tr h="{row_height}">"#));
        for cell in row {
            let (text, header) = cell.unwrap_or(("", false));
            let bold = if header { r#" b="1""# } else { "" };
            tbl.push_str(&format!(
                r#"<a:tc><a:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang="en-US" sz="1100"{bold}/><a:t>{}</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc>"#,
                xml_escape(text)
            ));
        }
        tbl.push_str("</a:tr>");
    }
    tbl.push_str("</a:tbl>");
    format!(
        r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="{id}" name="Table {id}"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{cx}" cy="{cy}"/></p:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table">{tbl}</a:graphicData></a:graphic></p:graphicFrame>"#
    )
}

fn map_bbox(bbox: [f64; 4], page_size: (f64, f64), slide_size: (i64, i64)) -> (i64, i64, i64, i64) {
    let bbox = if bbox_valid(bbox) {
        bbox
    } else {
        [
            36.0,
            page_size.1 - 96.0,
            page_size.0 - 36.0,
            page_size.1 - 36.0,
        ]
    };
    let page_w = page_size.0.max(1.0);
    let page_h = page_size.1.max(1.0);
    let x = (bbox[0].max(0.0) / page_w * slide_size.0 as f64).round() as i64;
    let y =
        ((page_h - bbox[3].max(bbox[1])).max(0.0) / page_h * slide_size.1 as f64).round() as i64;
    let cx = ((bbox[2] - bbox[0]).abs() / page_w * slide_size.0 as f64)
        .round()
        .max(12_700.0) as i64;
    let cy = ((bbox[3] - bbox[1]).abs() / page_h * slide_size.1 as f64)
        .round()
        .max(12_700.0) as i64;
    (x, y, cx, cy)
}

#[derive(Debug, Clone)]
struct XlsxInputSheet {
    name: String,
    rows: Vec<Vec<String>>,
}

#[derive(Debug, Clone)]
struct PptxInputSlide {
    size: PageSize,
    items: Vec<PptxInputItem>,
}

#[derive(Debug, Clone)]
enum PptxInputItem {
    Text {
        text: String,
        bbox: [f64; 4],
    },
    Table {
        rows: Vec<Vec<String>>,
        bbox: [f64; 4],
    },
    Image {
        bytes: Vec<u8>,
        extension: String,
        bbox: [f64; 4],
    },
}

fn office_blocks_to_pdf(blocks: &[OfficeBlock], options: &OfficeToPdfOptions) -> Result<Vec<u8>> {
    let mut flow = FlowDocument::new(options.page_size, options.margins);
    let body_style = TextStyle::unicode(11.0);
    let italic_style = TextStyle::unicode(11.0);
    let title_style = TextStyle::unicode(22.0);
    let heading_style = TextStyle::unicode(16.0);
    let paragraph = ParagraphStyle::new().line_height(1.25);
    for block in blocks {
        match block {
            OfficeBlock::Paragraph { spans, style } => {
                let text = spans
                    .iter()
                    .map(|span| span.text.as_str())
                    .collect::<String>();
                if text.trim().is_empty() {
                    continue;
                }
                match style {
                    ParagraphRole::Title => {
                        flow.add_paragraph(&text, &title_style, &paragraph)?;
                        flow.add_spacer(8.0);
                    }
                    ParagraphRole::Heading(level) => {
                        let style = if *level <= 1 {
                            &title_style
                        } else {
                            &heading_style
                        };
                        flow.add_paragraph(&text, style, &paragraph)?;
                        flow.add_spacer(6.0);
                    }
                    ParagraphRole::Normal => {
                        let run_style = if spans.iter().any(|span| span.italic) {
                            &italic_style
                        } else {
                            &body_style
                        };
                        flow.add_paragraph(&text, run_style, &paragraph)?;
                        flow.add_spacer(4.0);
                    }
                }
            }
            OfficeBlock::List { ordered, items } => {
                flow.add_list(
                    items.iter().map(String::as_str),
                    *ordered,
                    &body_style,
                    &paragraph,
                )?;
                flow.add_spacer(4.0);
            }
            OfficeBlock::Table(rows) => {
                if let Some(table) = table_builder_from_rows(rows, flow_content_width(options)) {
                    flow.add_table(&table)?;
                    flow.add_spacer(8.0);
                }
            }
            OfficeBlock::Image {
                bytes,
                extension,
                width_points,
                height_points,
            } => {
                let handle = register_office_image(flow.builder_mut(), bytes, extension)?;
                let max_width = flow_content_width(options);
                let scale = (max_width / width_points.max(1.0)).min(1.0);
                flow.add_image(handle, width_points * scale, height_points * scale)?;
                flow.add_spacer(8.0);
            }
        }
    }
    flow.into_builder().to_bytes()
}

fn xlsx_sheets_to_pdf(sheets: &[XlsxInputSheet], options: &OfficeToPdfOptions) -> Result<Vec<u8>> {
    let mut flow = FlowDocument::new(options.page_size.landscape(), options.margins);
    let content_width = flow_content_width(&OfficeToPdfOptions {
        page_size: options.page_size.landscape(),
        margins: options.margins,
    });
    let text_style = TextStyle::unicode(9.0);
    let heading_style = TextStyle::unicode(15.0);
    let paragraph = ParagraphStyle::new().line_height(1.2);
    for (sheet_idx, sheet) in sheets.iter().enumerate() {
        if sheet_idx > 0 {
            flow.add_page_break();
        }
        flow.add_paragraph(&sheet.name, &heading_style, &paragraph)?;
        flow.add_spacer(8.0);
        let max_cols = (content_width / 90.0).floor().max(1.0) as usize;
        let col_count = sheet.rows.iter().map(Vec::len).max().unwrap_or(0);
        if col_count == 0 {
            flow.add_paragraph("(empty sheet)", &text_style, &paragraph)?;
            continue;
        }
        for start_col in (0..col_count).step_by(max_cols) {
            let end_col = (start_col + max_cols).min(col_count);
            let chunk = sheet
                .rows
                .iter()
                .map(|row| {
                    (start_col..end_col)
                        .map(|col| row.get(col).cloned().unwrap_or_default())
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            if start_col > 0 {
                flow.add_page_break();
                flow.add_paragraph(
                    &format!("{} columns {}-{}", sheet.name, start_col + 1, end_col),
                    &heading_style,
                    &paragraph,
                )?;
                flow.add_spacer(8.0);
            }
            if let Some(table) = table_builder_from_rows(&chunk, content_width) {
                flow.add_table(&table)?;
            }
        }
    }
    flow.into_builder().to_bytes()
}

fn pptx_slides_to_pdf(slides: &[PptxInputSlide]) -> Result<Vec<u8>> {
    let mut builder = PdfBuilder::new();
    for slide in slides {
        builder.add_page(slide.size);
        for item in &slide.items {
            match item {
                PptxInputItem::Text { text, bbox } => {
                    let style = TextStyle::unicode(14.0);
                    let paragraph = ParagraphStyle::new().line_height(1.15);
                    let y = slide.size.height - bbox[1] - style.size;
                    let width = (bbox[2] - bbox[0]).max(36.0);
                    builder
                        .pages_mut()
                        .last_mut()
                        .expect("slide page")
                        .draw_paragraph(text, bbox[0], y, width, &style, &paragraph)?;
                }
                PptxInputItem::Table { rows, bbox } => {
                    if let Some(table) =
                        table_builder_from_rows(rows, (bbox[2] - bbox[0]).max(72.0))
                    {
                        let top_y = slide.size.height - bbox[1];
                        table.draw_on_page(
                            builder.pages_mut().last_mut().expect("slide page"),
                            bbox[0],
                            top_y,
                        )?;
                    }
                }
                PptxInputItem::Image {
                    bytes,
                    extension,
                    bbox,
                } => {
                    let handle = register_office_image(&mut builder, bytes, extension)?;
                    let width = (bbox[2] - bbox[0]).max(12.0);
                    let height = (bbox[3] - bbox[1]).max(12.0);
                    let y = slide.size.height - bbox[1] - height;
                    builder
                        .pages_mut()
                        .last_mut()
                        .expect("slide page")
                        .draw_image(handle, bbox[0], y, width, height);
                }
            }
        }
    }
    if slides.is_empty() {
        let mut flow = FlowDocument::new(PageSize::LETTER, Margins::all(54.0));
        flow.add_paragraph(
            "No slides found",
            &TextStyle::unicode(11.0),
            &ParagraphStyle::new(),
        )?;
        return flow.into_builder().to_bytes();
    }
    builder.to_bytes()
}

fn flow_content_width(options: &OfficeToPdfOptions) -> f64 {
    (options.page_size.width - options.margins.left - options.margins.right).max(1.0)
}

fn table_builder_from_rows(rows: &[Vec<String>], width: f64) -> Option<TableBuilder> {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    if cols == 0 {
        return None;
    }
    let col_width = (width / cols as f64).max(36.0);
    let mut builder = TableBuilder::new((0..cols).map(|_| TableColumn::new(col_width)).collect());
    builder = builder
        .body_style(TextStyle::unicode(9.0))
        .header_style(TextStyle::unicode(9.0));
    if let Some(header) = rows.first() {
        builder.set_header((0..cols).map(|idx| {
            header
                .get(idx)
                .map(|value| table_cell_pdf_text(value))
                .unwrap_or_default()
        }));
    }
    for row in rows.iter().skip(1) {
        builder.add_row((0..cols).map(|idx| {
            row.get(idx)
                .map(|value| table_cell_pdf_text(value))
                .unwrap_or_default()
        }));
    }
    Some(builder)
}

fn table_cell_pdf_text(value: &str) -> String {
    const MAX_CHARS: usize = 240;
    if value.chars().count() <= MAX_CHARS {
        return value.to_string();
    }
    let mut out = value.chars().take(MAX_CHARS).collect::<String>();
    out.push_str("...");
    out
}

fn register_office_image(
    builder: &mut PdfBuilder,
    bytes: &[u8],
    extension: &str,
) -> Result<ImageHandle> {
    if extension.eq_ignore_ascii_case("jpg") || extension.eq_ignore_ascii_case("jpeg") {
        builder.add_jpeg_image(bytes.to_vec())
    } else {
        builder.add_png_image(bytes)
    }
}

fn parse_docx_blocks(bytes: &[u8]) -> Result<Vec<OfficeBlock>> {
    let xml = zip_entry_string(bytes, "word/document.xml")?
        .ok_or_else(|| OxideError::MalformedPdf("docx: missing word/document.xml".to_string()))?;
    let rels = zip_entry_string(bytes, "word/_rels/document.xml.rels")?.unwrap_or_default();
    let rel_map = parse_relationships(&rels);
    let mut blocks = Vec::new();
    let mut pos = 0usize;
    while pos < xml.len() {
        let next_p = xml[pos..].find("<w:p").map(|idx| pos + idx);
        let next_tbl = xml[pos..].find("<w:tbl").map(|idx| pos + idx);
        let Some(start) = min_option(next_p, next_tbl) else {
            break;
        };
        if next_tbl == Some(start) {
            let Some(end) = find_close(&xml, start, "</w:tbl>") else {
                break;
            };
            let table_xml = &xml[start..end];
            blocks.push(OfficeBlock::Table(parse_docx_table(table_xml)));
            pos = end;
        } else {
            let Some(end) = find_close(&xml, start, "</w:p>") else {
                break;
            };
            let para_xml = &xml[start..end];
            append_docx_paragraph_blocks(para_xml, &rel_map, bytes, &mut blocks)?;
            pos = end;
        }
    }
    Ok(blocks)
}

fn append_docx_paragraph_blocks(
    para_xml: &str,
    rel_map: &HashMap<String, String>,
    package: &[u8],
    blocks: &mut Vec<OfficeBlock>,
) -> Result<()> {
    let spans = parse_docx_runs(para_xml);
    let text = spans
        .iter()
        .map(|span| span.text.as_str())
        .collect::<String>();
    if !text.trim().is_empty() {
        let role = if para_xml.contains(r#"<w:pStyle w:val="Title""#) {
            ParagraphRole::Title
        } else if para_xml.contains("Heading1") {
            ParagraphRole::Heading(1)
        } else if para_xml.contains("Heading2") {
            ParagraphRole::Heading(2)
        } else if para_xml.contains("Heading3") {
            ParagraphRole::Heading(3)
        } else {
            ParagraphRole::Normal
        };
        if para_xml.contains("<w:numPr>") || para_xml.contains("<w:numPr ") {
            let ordered = para_xml.contains(r#"<w:numId w:val="2""#);
            if let Some(OfficeBlock::List {
                ordered: last_ordered,
                items,
            }) = blocks.last_mut()
            {
                if *last_ordered == ordered {
                    items.push(text);
                } else {
                    blocks.push(OfficeBlock::List {
                        ordered,
                        items: vec![text],
                    });
                }
            } else {
                blocks.push(OfficeBlock::List {
                    ordered,
                    items: vec![text],
                });
            }
        } else {
            blocks.push(OfficeBlock::Paragraph { spans, style: role });
        }
    }

    for rel_id in collect_attr_values(para_xml, "r:embed") {
        let Some(target) = rel_map.get(&rel_id) else {
            continue;
        };
        let path = normalize_word_target(target);
        let Some(image_bytes) = zip_entry_bytes(package, &path)? else {
            continue;
        };
        let (width, height) = parse_docx_extent(para_xml).unwrap_or((216.0, 144.0));
        blocks.push(OfficeBlock::Image {
            bytes: image_bytes,
            extension: path.rsplit('.').next().unwrap_or("png").to_string(),
            width_points: width,
            height_points: height,
        });
    }
    Ok(())
}

fn parse_docx_runs(para_xml: &str) -> Vec<InlineSpan> {
    let mut spans = Vec::new();
    for run in split_elements(para_xml, "<w:r", "</w:r>") {
        let text = collect_tag_text(run, "w:t").join("");
        if text.is_empty() {
            continue;
        }
        spans.push(InlineSpan {
            text,
            bold: run.contains("<w:b") && !run.contains(r#"<w:b w:val="0""#),
            italic: run.contains("<w:i") && !run.contains(r#"<w:i w:val="0""#),
            link: None,
        });
    }
    if spans.is_empty() {
        let text = collect_tag_text(para_xml, "w:t").join("");
        if !text.is_empty() {
            spans.push(InlineSpan {
                text,
                ..Default::default()
            });
        }
    }
    spans
}

fn parse_docx_table(table_xml: &str) -> Vec<Vec<String>> {
    split_elements(table_xml, "<w:tr", "</w:tr>")
        .into_iter()
        .map(|row_xml| {
            split_elements(row_xml, "<w:tc", "</w:tc>")
                .into_iter()
                .map(|cell_xml| collect_tag_text(cell_xml, "w:t").join(""))
                .collect::<Vec<_>>()
        })
        .filter(|row| !row.is_empty())
        .collect()
}

fn parse_docx_extent(para_xml: &str) -> Option<(f64, f64)> {
    let extent = find_start_tag(para_xml, "wp:extent")?;
    let cx: f64 = attr_value(extent, "cx")?.parse().ok()?;
    let cy: f64 = attr_value(extent, "cy")?.parse().ok()?;
    Some((cx / 12_700.0, cy / 12_700.0))
}

fn parse_xlsx_sheets(bytes: &[u8]) -> Result<Vec<XlsxInputSheet>> {
    let shared_strings = zip_entry_string(bytes, "xl/sharedStrings.xml")?
        .map(|xml| parse_shared_strings(&xml))
        .unwrap_or_default();
    let mut names = zip_entry_names(bytes)?;
    names.retain(|name| name.starts_with("xl/worksheets/sheet") && name.ends_with(".xml"));
    names.sort_by_key(|name| {
        name.trim_start_matches("xl/worksheets/sheet")
            .trim_end_matches(".xml")
            .parse::<usize>()
            .unwrap_or(usize::MAX)
    });
    let mut sheets = Vec::new();
    for (idx, name) in names.into_iter().enumerate() {
        let Some(xml) = zip_entry_string(bytes, &name)? else {
            continue;
        };
        sheets.push(XlsxInputSheet {
            name: format!("Sheet {}", idx + 1),
            rows: parse_xlsx_sheet_rows(&xml, &shared_strings),
        });
    }
    Ok(sheets)
}

fn parse_shared_strings(xml: &str) -> Vec<String> {
    split_elements(xml, "<si", "</si>")
        .into_iter()
        .map(|si| collect_tag_text(si, "t").join(""))
        .collect()
}

fn parse_xlsx_sheet_rows(xml: &str, shared_strings: &[String]) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    for row_xml in split_elements(xml, "<row", "</row>") {
        let mut row = Vec::<String>::new();
        for cell_xml in split_elements(row_xml, "<c", "</c>") {
            let tag = find_start_tag(cell_xml, "c").unwrap_or("");
            let col = attr_value(tag, "r")
                .and_then(|r| column_index_from_cell_ref(&r))
                .unwrap_or(row.len() + 1);
            if row.len() < col {
                row.resize(col, String::new());
            }
            let raw = if cell_xml.contains(r#"t="s""#) {
                collect_tag_text(cell_xml, "v")
                    .first()
                    .and_then(|idx| idx.parse::<usize>().ok())
                    .and_then(|idx| shared_strings.get(idx).cloned())
                    .unwrap_or_default()
            } else if cell_xml.contains("inlineStr") {
                collect_tag_text(cell_xml, "t").join("")
            } else {
                collect_tag_text(cell_xml, "v").join("")
            };
            row[col - 1] = raw;
        }
        if row.iter().any(|cell| !cell.trim().is_empty()) {
            rows.push(row);
        }
    }
    rows
}

fn parse_pptx_slides(bytes: &[u8]) -> Result<Vec<PptxInputSlide>> {
    let presentation = zip_entry_string(bytes, "ppt/presentation.xml")?.unwrap_or_default();
    let (slide_w, slide_h) = parse_pptx_slide_size(&presentation);
    let mut names = zip_entry_names(bytes)?;
    names.retain(|name| name.starts_with("ppt/slides/slide") && name.ends_with(".xml"));
    names.sort_by_key(|name| {
        name.trim_start_matches("ppt/slides/slide")
            .trim_end_matches(".xml")
            .parse::<usize>()
            .unwrap_or(usize::MAX)
    });
    let mut slides = Vec::new();
    for name in names {
        let Some(xml) = zip_entry_string(bytes, &name)? else {
            continue;
        };
        let rel_path = name.replace("ppt/slides/", "ppt/slides/_rels/") + ".rels";
        let rels = zip_entry_string(bytes, &rel_path)?.unwrap_or_default();
        let rel_map = parse_relationships(&rels);
        slides.push(PptxInputSlide {
            size: PageSize::custom(slide_w, slide_h),
            items: parse_pptx_slide_items(&xml, &rel_map, bytes)?,
        });
    }
    Ok(slides)
}

fn parse_pptx_slide_size(xml: &str) -> (f64, f64) {
    let Some(tag) = find_start_tag(xml, "p:sldSz") else {
        return (720.0, 405.0);
    };
    let cx = attr_value(tag, "cx")
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(9_144_000.0);
    let cy = attr_value(tag, "cy")
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(5_143_500.0);
    (cx / 12_700.0, cy / 12_700.0)
}

fn parse_pptx_slide_items(
    xml: &str,
    rel_map: &HashMap<String, String>,
    package: &[u8],
) -> Result<Vec<PptxInputItem>> {
    let mut items = Vec::new();
    for frame in split_elements(xml, "<p:graphicFrame", "</p:graphicFrame>") {
        if !frame.contains("<a:tbl") {
            continue;
        }
        let rows = split_elements(frame, "<a:tr", "</a:tr>")
            .into_iter()
            .map(|row| {
                split_elements(row, "<a:tc", "</a:tc>")
                    .into_iter()
                    .map(|cell| collect_tag_text(cell, "a:t").join(""))
                    .collect::<Vec<_>>()
            })
            .filter(|row| !row.is_empty())
            .collect::<Vec<_>>();
        items.push(PptxInputItem::Table {
            rows,
            bbox: parse_pptx_bbox(frame),
        });
    }
    for shape in split_elements(xml, "<p:sp", "</p:sp>") {
        let text = collect_tag_text(shape, "a:t").join("\n");
        if !text.trim().is_empty() {
            items.push(PptxInputItem::Text {
                text,
                bbox: parse_pptx_bbox(shape),
            });
        }
    }
    for pic in split_elements(xml, "<p:pic", "</p:pic>") {
        let Some(blip) = find_start_tag(pic, "a:blip") else {
            continue;
        };
        let Some(rel_id) = attr_value(blip, "r:embed") else {
            continue;
        };
        let Some(target) = rel_map.get(&rel_id) else {
            continue;
        };
        let path = normalize_ppt_target(target);
        let Some(bytes) = zip_entry_bytes(package, &path)? else {
            continue;
        };
        items.push(PptxInputItem::Image {
            extension: path.rsplit('.').next().unwrap_or("png").to_string(),
            bytes,
            bbox: parse_pptx_bbox(pic),
        });
    }
    items.sort_by(|a, b| {
        let ay = ppt_item_bbox(a)[1];
        let by = ppt_item_bbox(b)[1];
        ay.partial_cmp(&by).unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(items)
}

fn parse_pptx_bbox(xml: &str) -> [f64; 4] {
    let off = find_start_tag(xml, "a:off").unwrap_or("");
    let ext = find_start_tag(xml, "a:ext").unwrap_or("");
    let x = attr_value(off, "x")
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(914_400.0)
        / 12_700.0;
    let y = attr_value(off, "y")
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(914_400.0)
        / 12_700.0;
    let w = attr_value(ext, "cx")
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(2_743_200.0)
        / 12_700.0;
    let h = attr_value(ext, "cy")
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(914_400.0)
        / 12_700.0;
    [x, y, x + w, y + h]
}

fn ppt_item_bbox(item: &PptxInputItem) -> [f64; 4] {
    match item {
        PptxInputItem::Text { bbox, .. }
        | PptxInputItem::Table { bbox, .. }
        | PptxInputItem::Image { bbox, .. } => *bbox,
    }
}

fn points_to_emu(points: f64) -> i64 {
    (points.max(1.0) * 12_700.0).round() as i64
}

fn sanitize_sheet_name(input: &str) -> String {
    let mut out = input
        .chars()
        .map(|c| match c {
            ':' | '\\' | '/' | '?' | '*' | '[' | ']' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect::<String>();
    out = out.trim().to_string();
    if out.is_empty() {
        out = "Sheet".to_string();
    }
    if out.chars().count() > 31 {
        out = out.chars().take(31).collect();
    }
    out
}

fn cell_ref(row: usize, col: usize) -> String {
    format!("{}{}", column_name(col), row)
}

fn column_name(mut col: usize) -> String {
    if col == 0 {
        return "A".to_string();
    }
    let mut chars = Vec::new();
    while col > 0 {
        col -= 1;
        chars.push((b'A' + (col % 26) as u8) as char);
        col /= 26;
    }
    chars.iter().rev().collect()
}

fn xml_escape(input: &str) -> String {
    input
        .chars()
        .filter_map(|c| match c {
            '&' => Some("&amp;".to_string()),
            '<' => Some("&lt;".to_string()),
            '>' => Some("&gt;".to_string()),
            '"' => Some("&quot;".to_string()),
            '\'' => Some("&apos;".to_string()),
            c if c.is_control() && c != '\n' && c != '\t' && c != '\r' => None,
            c => Some(c.to_string()),
        })
        .collect()
}

fn xml_unescape(input: &str) -> String {
    input
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn zip_entry_bytes(bytes: &[u8], name: &str) -> Result<Option<Vec<u8>>> {
    let mut zip = ZipArchive::new(Cursor::new(bytes)).map_err(zip_err)?;
    let result = match zip.by_name(name) {
        Ok(mut entry) => {
            let mut out = Vec::new();
            entry.read_to_end(&mut out)?;
            Ok(Some(out))
        }
        Err(zip::result::ZipError::FileNotFound) => Ok(None),
        Err(err) => Err(zip_err(err)),
    };
    result
}

fn zip_entry_string(bytes: &[u8], name: &str) -> Result<Option<String>> {
    let Some(entry) = zip_entry_bytes(bytes, name)? else {
        return Ok(None);
    };
    String::from_utf8(entry)
        .map(Some)
        .map_err(|err| OxideError::MalformedPdf(format!("{name}: XML is not UTF-8: {err}")))
}

fn zip_entry_names(bytes: &[u8]) -> Result<Vec<String>> {
    let mut zip = ZipArchive::new(Cursor::new(bytes)).map_err(zip_err)?;
    let mut names = Vec::new();
    for idx in 0..zip.len() {
        names.push(zip.by_index(idx).map_err(zip_err)?.name().to_string());
    }
    Ok(names)
}

fn split_elements<'a>(xml: &'a str, start_pat: &str, end_pat: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while let Some(start_rel) = xml[pos..].find(start_pat) {
        let start = pos + start_rel;
        let Some(end) = find_close(xml, start, end_pat) else {
            break;
        };
        out.push(&xml[start..end]);
        pos = end;
    }
    out
}

fn find_close(xml: &str, start: usize, end_pat: &str) -> Option<usize> {
    xml[start..]
        .find(end_pat)
        .map(|end_rel| start + end_rel + end_pat.len())
}

fn find_start_tag<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!("<{name}");
    let start = xml.find(&needle)?;
    let end = xml[start..].find('>')?;
    Some(&xml[start..start + end + 1])
}

fn attr_value(tag: &str, attr: &str) -> Option<String> {
    let double = format!(r#"{attr}=""#);
    if let Some(start) = tag.find(&double) {
        let value_start = start + double.len();
        let value_end = tag[value_start..].find('"')?;
        return Some(xml_unescape(&tag[value_start..value_start + value_end]));
    }
    let single = format!("{attr}='");
    let start = tag.find(&single)?;
    let value_start = start + single.len();
    let value_end = tag[value_start..].find('\'')?;
    Some(xml_unescape(&tag[value_start..value_start + value_end]))
}

fn collect_attr_values(xml: &str, attr: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    let needle = format!(r#"{attr}=""#);
    while let Some(start_rel) = xml[pos..].find(&needle) {
        let value_start = pos + start_rel + needle.len();
        let Some(value_end_rel) = xml[value_start..].find('"') else {
            break;
        };
        out.push(xml_unescape(&xml[value_start..value_start + value_end_rel]));
        pos = value_start + value_end_rel + 1;
    }
    out
}

fn collect_tag_text(xml: &str, tag: &str) -> Vec<String> {
    let mut out = Vec::new();
    let start_pat = format!("<{tag}");
    let end_pat = format!("</{tag}>");
    let mut pos = 0usize;
    while let Some(start_rel) = xml[pos..].find(&start_pat) {
        let start = pos + start_rel;
        let Some(open_end_rel) = xml[start..].find('>') else {
            break;
        };
        let content_start = start + open_end_rel + 1;
        let Some(close_rel) = xml[content_start..].find(&end_pat) else {
            break;
        };
        out.push(xml_unescape(&xml[content_start..content_start + close_rel]));
        pos = content_start + close_rel + end_pat.len();
    }
    out
}

fn parse_relationships(xml: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut pos = 0usize;
    while let Some(start_rel) = xml[pos..].find("<Relationship ") {
        let start = pos + start_rel;
        let Some(end_rel) = xml[start..].find('>') else {
            break;
        };
        let tag = &xml[start..start + end_rel + 1];
        if let (Some(id), Some(target)) = (attr_value(tag, "Id"), attr_value(tag, "Target")) {
            out.insert(id, target);
        }
        pos = start + end_rel + 1;
    }
    out
}

fn normalize_word_target(target: &str) -> String {
    if target.starts_with("word/") {
        target.to_string()
    } else if let Some(rest) = target.strip_prefix("../") {
        rest.to_string()
    } else {
        format!("word/{target}")
    }
}

fn normalize_ppt_target(target: &str) -> String {
    if target.starts_with("ppt/") {
        target.to_string()
    } else if let Some(rest) = target.strip_prefix("../") {
        format!("ppt/{rest}")
    } else {
        format!("ppt/slides/{target}")
    }
}

fn min_option(a: Option<usize>, b: Option<usize>) -> Option<usize> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

fn column_index_from_cell_ref(reference: &str) -> Option<usize> {
    let mut col = 0usize;
    let mut saw_letter = false;
    for ch in reference.chars() {
        if ch.is_ascii_alphabetic() {
            saw_letter = true;
            col = col * 26 + (ch.to_ascii_uppercase() as u8 - b'A' + 1) as usize;
        } else {
            break;
        }
    }
    saw_letter.then_some(col)
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::path::PathBuf;

    use zip::ZipArchive;

    use super::*;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(name)
    }

    fn zip_entry(bytes: &[u8], name: &str) -> String {
        let cursor = Cursor::new(bytes);
        let mut zip = ZipArchive::new(cursor).expect("open zip");
        let mut entry = zip.by_name(name).expect("zip entry");
        let mut out = String::new();
        entry.read_to_string(&mut out).expect("read entry");
        out
    }

    #[test]
    fn xlsx_column_names_continue_after_z() {
        assert_eq!(column_name(1), "A");
        assert_eq!(column_name(26), "Z");
        assert_eq!(column_name(27), "AA");
    }

    #[test]
    fn xlsx_writer_produces_openxml_package() {
        let engine = ContentEngine::open_path(fixture("tracemonkey.pdf")).expect("open fixture");
        let bytes = pdf_to_xlsx(&engine, &XlsxOptions::default()).expect("xlsx");
        let workbook = zip_entry(&bytes, "xl/workbook.xml");
        assert!(workbook.contains("<sheet name=\"Page 1\""));
        let sheet = zip_entry(&bytes, "xl/worksheets/sheet1.xml");
        assert!(sheet.contains("<worksheet"));
        assert!(sheet.contains("<sheetData>"));
    }

    #[test]
    fn pptx_writer_produces_slide_package() {
        let engine = ContentEngine::open_path(fixture("tracemonkey.pdf")).expect("open fixture");
        let bytes = pdf_to_pptx(&engine, &PptxOptions::default()).expect("pptx");
        let presentation = zip_entry(&bytes, "ppt/presentation.xml");
        assert!(presentation.contains("<p:sldIdLst>"));
        let slide = zip_entry(&bytes, "ppt/slides/slide1.xml");
        assert!(slide.contains("<p:sld"));
        assert!(slide.contains("<p:sp") || slide.contains("<p:graphicFrame"));
    }

    #[test]
    fn docx_writer_produces_word_package() {
        let engine = ContentEngine::open_path(fixture("tracemonkey.pdf")).expect("open fixture");
        let bytes = pdf_to_docx(&engine, &DocxOptions::default()).expect("docx");
        let document = zip_entry(&bytes, "word/document.xml");
        assert!(document.contains("<w:document"));
        assert!(document.contains("<w:p") || document.contains("<w:tbl"));
        let styles = zip_entry(&bytes, "word/styles.xml");
        assert!(styles.contains("Heading1"));
    }

    #[test]
    fn native_office_to_pdf_outputs_openable_pdfs() {
        let engine = ContentEngine::open_path(fixture("tracemonkey.pdf")).expect("open fixture");
        let docx = pdf_to_docx(&engine, &DocxOptions::default()).expect("docx");
        let xlsx = pdf_to_xlsx(&engine, &XlsxOptions::default()).expect("xlsx");
        let pptx = pdf_to_pptx(&engine, &PptxOptions::default()).expect("pptx");

        for pdf in [
            docx_to_pdf(&docx, &OfficeToPdfOptions::default()).expect("docx to pdf"),
            xlsx_to_pdf(&xlsx, &OfficeToPdfOptions::default()).expect("xlsx to pdf"),
            pptx_to_pdf(&pptx, &OfficeToPdfOptions::default()).expect("pptx to pdf"),
        ] {
            assert!(pdf.starts_with(b"%PDF-"));
            let reopened = ContentEngine::open_bytes(pdf).expect("generated pdf opens");
            assert!(reopened.page_count().expect("page count") >= 1);
        }
    }

    #[test]
    fn numeric_cells_are_written_as_numbers() {
        let xml = xlsx_cell_xml(
            1,
            &XlsxCell {
                col: 1,
                text: "$1,234.50".to_string(),
                style: 0,
            },
        );
        assert!(xml.contains("<v>1234.5</v>"));
        assert!(!xml.contains("inlineStr"));
    }
}
