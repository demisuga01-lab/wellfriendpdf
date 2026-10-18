//! PDF to EPUB 3 export (`pdf-to-epub`).
//!
//! Built from the canonical parsed document model ([`crate::parse::Document`])
//! that the Office and Markdown exporters already consume, so reading order,
//! headings, paragraphs, lists and tables come from the same extraction pass.
//! Images are the page's decodable image XObjects, re-encoded as PNG and placed
//! at their figure block (or, failing that, by vertical position in the page's
//! reading order).
//!
//! Package layout (EPUB 3.3 OCF):
//!
//! ```text
//! mimetype                      first entry, stored (uncompressed), no extras
//! META-INF/container.xml
//! OEBPS/content.opf             package document: metadata, manifest, spine
//! OEBPS/nav.xhtml               EPUB navigation document (toc + landmarks)
//! OEBPS/style.css
//! OEBPS/chapter-NNN.xhtml       one per chapter
//! OEBPS/images/image-<digest>.png
//! ```
//!
//! Chapters split on top-level headings (the document's highest heading rank;
//! a Title block counts as top level) or, with [`EpubSplit::Pages`], every N
//! pages. A document with no headings falls back to page-based splitting.
//! Output is deterministic: entry timestamps are fixed and the package
//! identifier is derived from the input bytes.

use std::collections::BTreeSet;
use std::io::{Cursor, Write};

use quick_xml::escape::escape;
use serde::{Deserialize, Serialize};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::analysis::tables::Table;
use crate::editable::{EditableBuildOptions, EditableDocument};
use crate::engine::{ContentEngine, ExtractionProfile};
use crate::error::{Result, WellfriendError};
use crate::parse::{Block, BlockKind, Document, InlineSpan, InlineText, ParseOptions};
use crate::versioning::resource_digest;
use crate::PageRegion;

/// Default pages per chapter for page-based splitting.
pub const DEFAULT_EPUB_PAGES_PER_CHAPTER: usize = 10;

/// How the book is divided into XHTML chapter files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum EpubSplit {
    /// New chapter at each top-level heading.
    Headings,
    /// New chapter every `n` PDF pages.
    Pages { n: usize },
}

impl EpubSplit {
    /// Parse `headings` or `pages` (with `pages_per_chapter`).
    pub fn parse(mode: &str, pages_per_chapter: Option<usize>) -> Option<Self> {
        match mode.trim().to_ascii_lowercase().as_str() {
            "" | "headings" | "heading" => Some(Self::Headings),
            "pages" | "page" => Some(Self::Pages {
                n: pages_per_chapter
                    .unwrap_or(DEFAULT_EPUB_PAGES_PER_CHAPTER)
                    .max(1),
            }),
            _ => None,
        }
    }
}

/// Options for [`pdf_to_epub`].
#[derive(Debug, Clone, PartialEq)]
pub struct EpubOptions {
    pub split: EpubSplit,
    /// Embed decodable images.
    pub include_images: bool,
    /// Keep running headers, footers and page numbers (off by default).
    pub include_furniture: bool,
    /// Book language (BCP 47). Defaults to the PDF `/Lang`, else `en`.
    pub language: Option<String>,
    /// Title override. Defaults to the PDF title, else the first heading.
    pub title: Option<String>,
    /// 1-based pages to include; empty means every page.
    pub pages: Vec<usize>,
}

impl Default for EpubOptions {
    fn default() -> Self {
        Self {
            split: EpubSplit::Headings,
            include_images: true,
            include_furniture: false,
            language: None,
            title: None,
            pages: Vec::new(),
        }
    }
}

/// JSON options shared by the CLI (`--options`), the HTTP route and the C API.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct EpubJsonOptions {
    /// `headings` (default) or `pages`.
    #[serde(alias = "chapterSplit", alias = "chapter_split")]
    pub split: Option<String>,
    /// Pages per chapter for `split = "pages"` (default 10).
    #[serde(alias = "pagesPerChapter")]
    pub pages_per_chapter: Option<usize>,
    /// 1-based page selection such as `1-5,8` (default: all pages).
    #[serde(alias = "pageRange", alias = "pages")]
    pub page_range: Option<String>,
    #[serde(alias = "includeImages")]
    pub include_images: Option<bool>,
    #[serde(alias = "includeFurniture")]
    pub include_furniture: Option<bool>,
    pub language: Option<String>,
    pub title: Option<String>,
    /// Password for encrypted inputs.
    pub password: Option<String>,
}

impl EpubJsonOptions {
    pub fn parse(options_json: Option<&str>) -> Result<Self> {
        match options_json.map(str::trim).filter(|s| !s.is_empty()) {
            None => Ok(Self::default()),
            Some(json) => {
                let value: serde_json::Value = serde_json::from_str(json).map_err(|err| {
                    WellfriendError::ParseError(format!("invalid_json: pdf-to-epub options: {err}"))
                })?;
                serde_json::from_value(value).map_err(|err| {
                    WellfriendError::ParseError(format!(
                        "invalid_options: pdf-to-epub options: {err}"
                    ))
                })
            }
        }
    }

    pub fn to_options(&self) -> Result<EpubOptions> {
        let split = EpubSplit::parse(
            self.split.as_deref().unwrap_or("headings"),
            self.pages_per_chapter,
        )
        .ok_or_else(|| {
            WellfriendError::ParseError(
                "invalid_options: split must be 'headings' or 'pages'".to_string(),
            )
        })?;
        let defaults = EpubOptions::default();
        Ok(EpubOptions {
            split,
            include_images: self.include_images.unwrap_or(defaults.include_images),
            include_furniture: self.include_furniture.unwrap_or(defaults.include_furniture),
            language: self.language.clone(),
            title: self.title.clone(),
            pages: Vec::new(),
        })
    }
}

/// Parse a 1-based page selection (`all`, `3`, `1-5`, `1,3,7-9`).
pub fn parse_epub_page_range(spec: &str, total: usize) -> Result<Vec<usize>> {
    let invalid = || {
        WellfriendError::ParseError(format!(
            "invalid_options: pageRange '{spec}' is not a valid page selection for {total} page(s)"
        ))
    };
    let spec = spec.trim();
    if spec.is_empty() || spec.eq_ignore_ascii_case("all") {
        return Ok((1..=total).collect());
    }
    let mut pages = Vec::new();
    for part in spec.split(',') {
        let part = part.trim();
        let (start, end) = match part.split_once('-') {
            Some((a, b)) => {
                let start: usize = a.trim().parse().map_err(|_| invalid())?;
                let end: usize = if b.trim().is_empty() {
                    total
                } else {
                    b.trim().parse().map_err(|_| invalid())?
                };
                (start, end)
            }
            None => {
                let page: usize = part.parse().map_err(|_| invalid())?;
                (page, page)
            }
        };
        if start == 0 || end < start || end > total {
            return Err(invalid());
        }
        for page in start..=end {
            if !pages.contains(&page) {
                pages.push(page);
            }
        }
    }
    pages.sort_unstable();
    Ok(pages)
}

/// Open `input` (with the optional password from the options) and convert it.
pub fn pdf_to_epub_json(input: &[u8], options_json: Option<&str>) -> Result<(Vec<u8>, EpubReport)> {
    let parsed = EpubJsonOptions::parse(options_json)?;
    let options = parsed.to_options()?;
    let engine = match parsed.password.as_deref() {
        Some(password) => {
            ContentEngine::open_bytes_with_password(input.to_vec(), password.as_bytes())?
        }
        None => ContentEngine::open_bytes(input.to_vec())?,
    };
    let mut options = options;
    if let Some(range) = parsed.page_range.as_deref() {
        options.pages = parse_epub_page_range(range, engine.page_count()?)?;
    }
    pdf_to_epub_with_report(&engine, &options)
}

/// Summary of a generated EPUB.
#[derive(Debug, Clone, Serialize)]
pub struct EpubReport {
    pub title: String,
    pub language: String,
    pub identifier: String,
    pub split: EpubSplit,
    pub chapters: Vec<EpubChapterReport>,
    pub images: usize,
    pub pages: usize,
    pub output_bytes: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct EpubChapterReport {
    pub href: String,
    pub title: String,
    pub first_page: u32,
    pub last_page: u32,
}

/// Convert a PDF to EPUB 3 bytes.
pub fn pdf_to_epub(engine: &ContentEngine, options: &EpubOptions) -> Result<Vec<u8>> {
    pdf_to_epub_with_report(engine, options).map(|(bytes, _)| bytes)
}

#[derive(Debug, Clone)]
struct EpubImage {
    page: u32,
    bbox: [f64; 4],
    href: String,
    width: f64,
}

#[derive(Debug, Clone)]
enum Item<'a> {
    Block(&'a Block),
    Image(usize),
}

#[derive(Debug, Default)]
struct Chapter<'a> {
    title: Option<String>,
    first_page: u32,
    last_page: u32,
    items: Vec<Item<'a>>,
}

/// Convert a PDF to EPUB 3 bytes and return a report of the package.
pub fn pdf_to_epub_with_report(
    engine: &ContentEngine,
    options: &EpubOptions,
) -> Result<(Vec<u8>, EpubReport)> {
    let parsed = engine.parse_document_with_profile(
        ExtractionProfile::LayoutFaithful,
        &ParseOptions {
            pages: options.pages.clone(),
            ..ParseOptions::default()
        },
    )?;
    let document =
        EditableDocument::from_parse_document(engine, parsed, &EditableBuildOptions::default())
            .to_parse_document();

    let mut image_bytes: Vec<(String, Vec<u8>)> = Vec::new();
    let images = if options.include_images {
        collect_images(engine, &document, &mut image_bytes)
    } else {
        Vec::new()
    };

    let mut blocks: Vec<&Block> = document
        .body
        .iter()
        .filter(|block| options.include_furniture || !block.kind.is_furniture())
        .collect();
    blocks.sort_by_key(|block| block.reading_order);
    let ordered = interleave_images(&blocks, &images);

    let split = match options.split {
        EpubSplit::Headings if top_heading_rank(&blocks).is_none() => EpubSplit::Pages {
            n: DEFAULT_EPUB_PAGES_PER_CHAPTER,
        },
        other => other,
    };
    let chapters = split_chapters(ordered, split, &blocks, &images);

    let metadata = &document.metadata;
    let title = options
        .title
        .clone()
        .filter(|t| !t.trim().is_empty())
        .or_else(|| metadata.title.clone().filter(|t| !t.trim().is_empty()))
        .or_else(|| chapters.iter().find_map(|chapter| chapter.title.clone()))
        .unwrap_or_else(|| "Untitled".to_string());
    let language = options
        .language
        .clone()
        .filter(|l| !l.trim().is_empty())
        .or_else(|| catalog_language(engine))
        .unwrap_or_else(|| "en".to_string());
    let identifier = format!("urn:uuid:{}", uuid_from_digest(engine));

    let mut chapter_reports = Vec::new();
    let mut chapter_files = Vec::new();
    for (index, chapter) in chapters.iter().enumerate() {
        let href = format!("chapter-{:03}.xhtml", index + 1);
        let chapter_title = chapter.title.clone().unwrap_or_else(|| {
            if chapter.first_page == chapter.last_page {
                format!("Page {}", chapter.first_page)
            } else {
                format!("Pages {}-{}", chapter.first_page, chapter.last_page)
            }
        });
        let xhtml = chapter_xhtml(chapter, &chapter_title, &language, &images);
        chapter_reports.push(EpubChapterReport {
            href: href.clone(),
            title: chapter_title,
            first_page: chapter.first_page,
            last_page: chapter.last_page,
        });
        chapter_files.push((href, xhtml));
    }

    let opf = package_opf(
        &title,
        &language,
        &identifier,
        metadata,
        &chapter_reports,
        &image_bytes,
    );
    let nav = nav_xhtml(&title, &language, &chapter_reports);

    let mut cursor = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut cursor);
        let stored = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .last_modified_time(zip::DateTime::default());
        let deflated = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .last_modified_time(zip::DateTime::default());
        write_entry(&mut zip, stored, "mimetype", b"application/epub+zip")?;
        write_entry(
            &mut zip,
            deflated,
            "META-INF/container.xml",
            CONTAINER_XML.as_bytes(),
        )?;
        write_entry(&mut zip, deflated, "OEBPS/content.opf", opf.as_bytes())?;
        write_entry(&mut zip, deflated, "OEBPS/nav.xhtml", nav.as_bytes())?;
        write_entry(&mut zip, deflated, "OEBPS/style.css", STYLE_CSS.as_bytes())?;
        for (href, xhtml) in &chapter_files {
            write_entry(
                &mut zip,
                deflated,
                &format!("OEBPS/{href}"),
                xhtml.as_bytes(),
            )?;
        }
        for (href, bytes) in &image_bytes {
            // PNG is already compressed.
            write_entry(&mut zip, stored, &format!("OEBPS/{href}"), bytes)?;
        }
        zip.finish().map_err(zip_err)?;
    }
    let bytes = cursor.into_inner();
    let report = EpubReport {
        title,
        language,
        identifier,
        split,
        chapters: chapter_reports,
        images: image_bytes.len(),
        pages: document.pages.len(),
        output_bytes: bytes.len(),
    };
    Ok((bytes, report))
}

fn write_entry<W: Write + std::io::Seek>(
    zip: &mut ZipWriter<W>,
    options: SimpleFileOptions,
    name: &str,
    bytes: &[u8],
) -> Result<()> {
    zip.start_file(name, options).map_err(zip_err)?;
    zip.write_all(bytes)?;
    Ok(())
}

fn zip_err(err: zip::result::ZipError) -> WellfriendError {
    WellfriendError::Io(std::io::Error::other(err.to_string()))
}

fn catalog_language(engine: &ContentEngine) -> Option<String> {
    let catalog = engine.document().get_catalog().ok()?;
    let lang = engine
        .document()
        .reader()
        .resolve(catalog.get("Lang")?.clone())
        .ok()?;
    let text = crate::info::decode_pdf_text_string(lang.as_string()?);
    let text = text.trim();
    let valid = !text.is_empty()
        && text.len() <= 35
        && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    valid.then(|| text.to_string())
}

fn uuid_from_digest(engine: &ContentEngine) -> String {
    let digest = resource_digest(engine.document().reader().file_bytes());
    let hex: Vec<char> = digest.chars().filter(char::is_ascii_hexdigit).collect();
    let mut chars: Vec<char> = hex.into_iter().take(32).collect();
    while chars.len() < 32 {
        chars.push('0');
    }
    // RFC 4122 version 5-style layout (name-based, SHA digest).
    chars[12] = '5';
    chars[16] = match chars[16].to_digit(16).unwrap_or(0) & 0x3 {
        0 => '8',
        1 => '9',
        2 => 'a',
        _ => 'b',
    };
    let s: String = chars.into_iter().collect::<String>().to_ascii_lowercase();
    format!(
        "{}-{}-{}-{}-{}",
        &s[0..8],
        &s[8..12],
        &s[12..16],
        &s[16..20],
        &s[20..32]
    )
}

fn collect_images(
    engine: &ContentEngine,
    document: &Document,
    store: &mut Vec<(String, Vec<u8>)>,
) -> Vec<EpubImage> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for page in &document.pages {
        let Ok(region) = PageRegion::new(0.0, 0.0, page.width.max(1.0), page.height.max(1.0))
        else {
            continue;
        };
        let Ok(found) = engine.find_page_images_in_region(page.number as usize, region) else {
            continue;
        };
        for image in found
            .into_iter()
            .filter(|img| !img.image.is_mask && !img.image.is_smask)
        {
            let Ok(bytes) = engine.extract_image_png_for_package(&image.image) else {
                continue;
            };
            let digest = resource_digest(&bytes);
            let suffix: String = digest
                .chars()
                .filter(char::is_ascii_alphanumeric)
                .take(16)
                .collect();
            let href = format!("images/image-{suffix}.png");
            if seen.insert(href.clone()) {
                store.push((href.clone(), bytes));
            }
            out.push(EpubImage {
                page: page.number,
                bbox: image.bbox,
                href,
                width: (image.bbox[2] - image.bbox[0]).abs(),
            });
        }
    }
    out
}

fn bbox_overlap(a: [f64; 4], b: [f64; 4]) -> f64 {
    let w = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let h = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    w * h
}

/// Merge images into the reading-ordered block stream: an image goes with
/// the figure block it overlaps most; otherwise after the last block on its
/// page whose top edge is above the image's top edge.
fn interleave_images<'a>(blocks: &[&'a Block], images: &[EpubImage]) -> Vec<Item<'a>> {
    let mut after_block: Vec<Vec<usize>> = vec![Vec::new(); blocks.len()];
    let mut page_start: Vec<(u32, usize)> = Vec::new();
    let mut leading: Vec<(u32, usize)> = Vec::new();
    for (index, image) in images.iter().enumerate() {
        let figure = blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| b.page == image.page && matches!(b.kind, BlockKind::Figure { .. }))
            .map(|(i, b)| (i, bbox_overlap(b.bbox, image.bbox)))
            .filter(|(_, overlap)| *overlap > 0.0)
            .max_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((block_index, _)) = figure {
            after_block[block_index].push(index);
            continue;
        }
        let anchor = blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| b.page == image.page && b.bbox[3] >= image.bbox[3])
            .map(|(i, _)| i)
            .max();
        match anchor {
            Some(block_index) => after_block[block_index].push(index),
            None => leading.push((image.page, index)),
        }
    }
    for (index, block) in blocks.iter().enumerate() {
        if !page_start.iter().any(|(page, _)| *page == block.page) {
            page_start.push((block.page, index));
        }
    }
    let mut items = Vec::new();
    let mut emitted_leading = BTreeSet::new();
    for (index, block) in blocks.iter().enumerate() {
        if page_start.contains(&(block.page, index)) {
            for (page, image) in &leading {
                if *page == block.page && emitted_leading.insert(*image) {
                    items.push(Item::Image(*image));
                }
            }
        }
        items.push(Item::Block(block));
        items.extend(after_block[index].iter().map(|i| Item::Image(*i)));
    }
    // Images on pages without any text block.
    for (_, image) in &leading {
        if emitted_leading.insert(*image) {
            items.push(Item::Image(*image));
        }
    }
    items
}

/// Highest heading rank present: 0 for Title, else the smallest level.
fn top_heading_rank(blocks: &[&Block]) -> Option<u8> {
    blocks.iter().filter_map(|block| heading_rank(block)).min()
}

fn heading_rank(block: &Block) -> Option<u8> {
    match &block.kind {
        BlockKind::Title { text } if !text.is_empty() => Some(0),
        BlockKind::Heading { level, text } if !text.is_empty() => Some(*level),
        _ => None,
    }
}

fn item_page(item: &Item<'_>, images: &[EpubImage], fallback: u32) -> u32 {
    match item {
        Item::Block(block) => block.page,
        Item::Image(index) => images.get(*index).map_or(fallback, |image| image.page),
    }
}

fn split_chapters<'a>(
    items: Vec<Item<'a>>,
    split: EpubSplit,
    blocks: &[&'a Block],
    images: &[EpubImage],
) -> Vec<Chapter<'a>> {
    let top = top_heading_rank(blocks);
    let mut chapters: Vec<Chapter<'a>> = Vec::new();
    let mut current = Chapter::default();
    let mut last_page = 1u32;
    for item in items {
        let page = item_page(&item, images, last_page);
        let starts_chapter = match (&item, split) {
            (Item::Block(block), EpubSplit::Headings) => {
                heading_rank(block).is_some() && heading_rank(block) == top
            }
            (_, EpubSplit::Pages { n }) => {
                !current.items.is_empty()
                    && (page.saturating_sub(1) as usize) / n
                        != (current.first_page.saturating_sub(1) as usize) / n
            }
            _ => false,
        };
        if starts_chapter && !current.items.is_empty() {
            chapters.push(std::mem::take(&mut current));
        }
        if current.items.is_empty() {
            current.first_page = page;
            if let (Item::Block(block), EpubSplit::Headings) = (&item, split) {
                if heading_rank(block).is_some() {
                    current.title = Some(clean_text(&block_plain(block)));
                }
            }
        }
        current.last_page = current.last_page.max(page);
        last_page = page;
        current.items.push(item);
    }
    if !current.items.is_empty() || chapters.is_empty() {
        if current.first_page == 0 {
            current.first_page = 1;
            current.last_page = 1;
        }
        chapters.push(current);
    }
    chapters
}

fn block_plain(block: &Block) -> String {
    match &block.kind {
        BlockKind::Title { text }
        | BlockKind::Heading { text, .. }
        | BlockKind::Paragraph { text }
        | BlockKind::Caption { text, .. }
        | BlockKind::Header { text }
        | BlockKind::Footer { text }
        | BlockKind::PageNumber { text }
        | BlockKind::Text { text } => text.to_plain(),
        _ => String::new(),
    }
}

/// Strip characters XML 1.0 forbids and collapse whitespace runs.
fn clean_text(text: &str) -> String {
    let filtered: String = text
        .chars()
        .filter(|c| {
            matches!(c, '\t' | '\n' | '\r')
                || (*c >= '\u{20}' && *c != '\u{FFFE}' && *c != '\u{FFFF}')
        })
        .collect();
    filtered.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn xml_text(text: &str) -> String {
    escape(clean_text(text).as_str()).into_owned()
}

fn inline_xhtml(text: &InlineText) -> String {
    let mut out = String::new();
    for (index, span) in text.spans.iter().enumerate() {
        let mut piece = span_text(span);
        if index > 0 && needs_space(&out, &piece) {
            out.push(' ');
        }
        if span.bold {
            piece = format!("<strong>{piece}</strong>");
        }
        if span.italic {
            piece = format!("<em>{piece}</em>");
        }
        if let Some(link) = span.link.as_deref().filter(|link| safe_link(link)) {
            piece = format!("<a href=\"{}\">{piece}</a>", escape(link));
        }
        out.push_str(&piece);
    }
    out
}

fn span_text(span: &InlineSpan) -> String {
    xml_text(&span.text)
}

fn needs_space(previous: &str, next: &str) -> bool {
    !previous.is_empty()
        && !previous.ends_with(' ')
        && !next.starts_with(' ')
        && !next.starts_with(['.', ',', ';', ':', '!', '?', ')'])
}

fn safe_link(link: &str) -> bool {
    let lower = link.to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:")
}

fn table_xhtml(table: &Table) -> String {
    let mut out = String::from("<table>\n");
    if table.cells.is_empty() {
        for (row_index, row) in table.rows.iter().enumerate() {
            out.push_str("<tr>");
            for cell in row {
                let tag = if row_index == 0 { "th" } else { "td" };
                out.push_str(&format!("<{tag}>{}</{tag}>", xml_text(cell)));
            }
            out.push_str("</tr>\n");
        }
    } else {
        let rows = table.num_rows().max(1);
        for row in 0..rows {
            let mut cells: Vec<_> = table.cells.iter().filter(|c| c.row == row).collect();
            cells.sort_by_key(|c| c.col);
            out.push_str("<tr>");
            for cell in cells {
                let tag = if cell.is_header { "th" } else { "td" };
                let mut attrs = String::new();
                if cell.rowspan > 1 {
                    attrs.push_str(&format!(" rowspan=\"{}\"", cell.rowspan));
                }
                if cell.colspan > 1 {
                    attrs.push_str(&format!(" colspan=\"{}\"", cell.colspan));
                }
                out.push_str(&format!("<{tag}{attrs}>{}</{tag}>", xml_text(&cell.text)));
            }
            out.push_str("</tr>\n");
        }
    }
    out.push_str("</table>\n");
    out
}

fn image_xhtml(image: &EpubImage, alt: &str) -> String {
    // Points to CSS px (96/72), capped by the stylesheet's max-width.
    let width = (image.width * 96.0 / 72.0).round().max(1.0);
    format!(
        "<figure><img src=\"{}\" alt=\"{}\" style=\"width:{width}px\"/></figure>\n",
        escape(image.href.as_str()),
        escape(alt)
    )
}

fn chapter_xhtml(
    chapter: &Chapter<'_>,
    title: &str,
    language: &str,
    images: &[EpubImage],
) -> String {
    let mut body = String::new();
    for item in &chapter.items {
        match item {
            Item::Image(index) => {
                if let Some(image) = images.get(*index) {
                    body.push_str(&image_xhtml(image, ""));
                }
            }
            Item::Block(block) => body.push_str(&block_xhtml(block)),
        }
    }
    if body.is_empty() {
        body.push_str("<p></p>\n");
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\" xml:lang=\"{lang}\" lang=\"{lang}\">\n<head>\n<meta charset=\"UTF-8\"/>\n<title>{title}</title>\n<link rel=\"stylesheet\" type=\"text/css\" href=\"style.css\"/>\n</head>\n<body>\n<section epub:type=\"chapter\">\n{body}</section>\n</body>\n</html>\n",
        lang = escape(language),
        title = xml_text(title),
    )
}

fn block_xhtml(block: &Block) -> String {
    match &block.kind {
        BlockKind::Title { text } => {
            if text.is_empty() {
                String::new()
            } else {
                format!("<h1>{}</h1>\n", inline_xhtml(text))
            }
        }
        BlockKind::Heading { level, text } => {
            if text.is_empty() {
                return String::new();
            }
            let level = (*level).clamp(1, 6);
            format!("<h{level}>{}</h{level}>\n", inline_xhtml(text))
        }
        BlockKind::Paragraph { text } | BlockKind::Text { text } => {
            if text.is_empty() {
                String::new()
            } else {
                format!("<p>{}</p>\n", inline_xhtml(text))
            }
        }
        BlockKind::Caption { text, .. } => {
            if text.is_empty() {
                String::new()
            } else {
                format!("<p class=\"caption\">{}</p>\n", inline_xhtml(text))
            }
        }
        BlockKind::Header { text }
        | BlockKind::Footer { text }
        | BlockKind::PageNumber { text } => {
            if text.is_empty() {
                String::new()
            } else {
                format!("<p class=\"furniture\">{}</p>\n", inline_xhtml(text))
            }
        }
        BlockKind::List { ordered, items } => {
            if items.is_empty() {
                return String::new();
            }
            let tag = if *ordered { "ol" } else { "ul" };
            let mut out = format!("<{tag}>\n");
            for item in items {
                out.push_str(&format!("<li>{}</li>\n", inline_xhtml(&item.text)));
            }
            out.push_str(&format!("</{tag}>\n"));
            out
        }
        BlockKind::Table { table, .. } => table_xhtml(table),
        BlockKind::Figure { alt, .. } => alt
            .as_deref()
            .filter(|alt| !alt.trim().is_empty())
            .map(|alt| format!("<p class=\"figure-alt\">{}</p>\n", xml_text(alt)))
            .unwrap_or_default(),
    }
}

fn package_opf(
    title: &str,
    language: &str,
    identifier: &str,
    metadata: &crate::parse::DocumentMetadata,
    chapters: &[EpubChapterReport],
    images: &[(String, Vec<u8>)],
) -> String {
    let mut meta = String::new();
    meta.push_str(&format!(
        "    <dc:identifier id=\"bookid\">{}</dc:identifier>\n",
        escape(identifier)
    ));
    meta.push_str(&format!("    <dc:title>{}</dc:title>\n", xml_text(title)));
    meta.push_str(&format!(
        "    <dc:language>{}</dc:language>\n",
        escape(language)
    ));
    if let Some(author) = metadata.author.as_deref().filter(|a| !a.trim().is_empty()) {
        for name in author.split(';').map(str::trim).filter(|n| !n.is_empty()) {
            meta.push_str(&format!(
                "    <dc:creator>{}</dc:creator>\n",
                xml_text(name)
            ));
        }
    }
    if let Some(subject) = metadata.subject.as_deref().filter(|s| !s.trim().is_empty()) {
        meta.push_str(&format!(
            "    <dc:description>{}</dc:description>\n",
            xml_text(subject)
        ));
    }
    if let Some(keywords) = metadata.keywords.as_deref() {
        for keyword in keywords
            .split([',', ';'])
            .map(str::trim)
            .filter(|k| !k.is_empty())
        {
            meta.push_str(&format!(
                "    <dc:subject>{}</dc:subject>\n",
                xml_text(keyword)
            ));
        }
    }
    let created = metadata.creation_date.as_deref().and_then(pdf_date_utc);
    let modified = metadata
        .modification_date
        .as_deref()
        .and_then(pdf_date_utc)
        .or(created.clone())
        .unwrap_or_else(|| "2000-01-01T00:00:00Z".to_string());
    if let Some(created) = created {
        meta.push_str(&format!("    <dc:date>{created}</dc:date>\n"));
    }
    if let Some(producer) = metadata
        .producer
        .as_deref()
        .or(metadata.creator.as_deref())
        .filter(|p| !p.trim().is_empty())
    {
        meta.push_str(&format!(
            "    <dc:contributor>{}</dc:contributor>\n",
            xml_text(producer)
        ));
    }
    meta.push_str(&format!(
        "    <meta property=\"dcterms:modified\">{modified}</meta>\n"
    ));

    let mut manifest = String::from(
        "    <item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>\n    <item id=\"css\" href=\"style.css\" media-type=\"text/css\"/>\n",
    );
    let mut spine = String::new();
    for (index, chapter) in chapters.iter().enumerate() {
        manifest.push_str(&format!(
            "    <item id=\"chapter-{n}\" href=\"{href}\" media-type=\"application/xhtml+xml\"/>\n",
            n = index + 1,
            href = escape(chapter.href.as_str())
        ));
        spine.push_str(&format!("    <itemref idref=\"chapter-{}\"/>\n", index + 1));
    }
    for (index, (href, _)) in images.iter().enumerate() {
        manifest.push_str(&format!(
            "    <item id=\"img-{n}\" href=\"{href}\" media-type=\"image/png\"/>\n",
            n = index + 1,
            href = escape(href.as_str())
        ));
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"bookid\" xml:lang=\"{lang}\">\n  <metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n{meta}  </metadata>\n  <manifest>\n{manifest}  </manifest>\n  <spine>\n{spine}  </spine>\n</package>\n",
        lang = escape(language)
    )
}

/// Convert a PDF date string to an XMP/OPF UTC timestamp (`...Z`).
fn pdf_date_utc(raw: &str) -> Option<String> {
    let text = raw.trim();
    let text = text.strip_prefix("D:").unwrap_or(text);
    let num = |start: usize, len: usize| -> Option<i64> {
        let slice = text.get(start..start + len)?;
        slice
            .bytes()
            .all(|b| b.is_ascii_digit())
            .then(|| slice.parse().ok())
            .flatten()
    };
    let year = num(0, 4)?;
    let month = num(4, 2).unwrap_or(1);
    let day = num(6, 2).unwrap_or(1);
    let hour = num(8, 2).unwrap_or(0);
    let minute = num(10, 2).unwrap_or(0);
    let second = num(12, 2).unwrap_or(0);
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let tail = text.get(14..).unwrap_or("");
    let offset_minutes = match tail.chars().next() {
        Some(sign @ ('+' | '-')) => {
            let tz = tail[1..].replace('\'', "");
            let h: i64 = tz.get(0..2)?.parse().ok()?;
            let m: i64 = tz.get(2..4).and_then(|v| v.parse().ok()).unwrap_or(0);
            if sign == '-' {
                -(h * 60 + m)
            } else {
                h * 60 + m
            }
        }
        _ => 0,
    };
    // Days-from-civil arithmetic to apply the offset across date boundaries.
    let days = days_from_civil(year, month, day);
    let total = days * 86_400 + hour * 3_600 + minute * 60 + second - offset_minutes * 60;
    let (days, secs) = (total.div_euclid(86_400), total.rem_euclid(86_400));
    let (y, mo, d) = civil_from_days(days);
    Some(format!(
        "{y:04}-{mo:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs / 3_600,
        (secs % 3_600) / 60,
        secs % 60
    ))
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

fn nav_xhtml(title: &str, language: &str, chapters: &[EpubChapterReport]) -> String {
    let mut toc = String::new();
    for chapter in chapters {
        toc.push_str(&format!(
            "      <li><a href=\"{}\">{}</a></li>\n",
            escape(chapter.href.as_str()),
            xml_text(&chapter.title)
        ));
    }
    let first = chapters
        .first()
        .map(|c| c.href.as_str())
        .unwrap_or("chapter-001.xhtml");
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\" xml:lang=\"{lang}\" lang=\"{lang}\">\n<head>\n<meta charset=\"UTF-8\"/>\n<title>{title}</title>\n<link rel=\"stylesheet\" type=\"text/css\" href=\"style.css\"/>\n</head>\n<body>\n  <nav epub:type=\"toc\" id=\"toc\">\n    <h1>Contents</h1>\n    <ol>\n{toc}    </ol>\n  </nav>\n  <nav epub:type=\"landmarks\" id=\"landmarks\" hidden=\"hidden\">\n    <ol>\n      <li><a epub:type=\"toc\" href=\"nav.xhtml\">Contents</a></li>\n      <li><a epub:type=\"bodymatter\" href=\"{first}\">Start</a></li>\n    </ol>\n  </nav>\n</body>\n</html>\n",
        lang = escape(language),
        title = xml_text(title),
        first = escape(first),
    )
}

const CONTAINER_XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\">\n  <rootfiles>\n    <rootfile full-path=\"OEBPS/content.opf\" media-type=\"application/oebps-package+xml\"/>\n  </rootfiles>\n</container>\n";

const STYLE_CSS: &str = "body { margin: 0 1em; line-height: 1.45; }\nh1, h2, h3, h4, h5, h6 { line-height: 1.2; page-break-after: avoid; }\nimg { max-width: 100%; height: auto; }\nfigure { margin: 1em 0; text-align: center; }\ntable { border-collapse: collapse; margin: 1em 0; }\nth, td { border: 1px solid #999; padding: 0.2em 0.4em; vertical-align: top; }\nth { background: #eee; }\n.caption { font-style: italic; text-align: center; }\n.furniture { color: #666; font-size: 0.85em; }\n";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_dates_normalise_to_utc() {
        assert_eq!(
            pdf_date_utc("D:20240101003000+05'30'").as_deref(),
            Some("2023-12-31T19:00:00Z")
        );
        assert_eq!(
            pdf_date_utc("D:20240229").as_deref(),
            Some("2024-02-29T00:00:00Z")
        );
        assert!(pdf_date_utc("garbage").is_none());
    }

    #[test]
    fn split_mode_parses() {
        assert_eq!(
            EpubSplit::parse("headings", None),
            Some(EpubSplit::Headings)
        );
        assert_eq!(
            EpubSplit::parse("pages", Some(3)),
            Some(EpubSplit::Pages { n: 3 })
        );
        assert_eq!(
            EpubSplit::parse("pages", Some(0)),
            Some(EpubSplit::Pages { n: 1 })
        );
        assert!(EpubSplit::parse("chapters", None).is_none());
    }

    #[test]
    fn text_is_escaped_and_cleaned() {
        assert_eq!(xml_text("a <b> & \u{1}c"), "a &lt;b&gt; &amp; c");
        let spans = InlineText {
            spans: vec![
                InlineSpan {
                    text: "Bold".into(),
                    bold: true,
                    ..Default::default()
                },
                InlineSpan {
                    text: "link".into(),
                    link: Some("javascript:alert(1)".into()),
                    ..Default::default()
                },
            ],
        };
        let html = inline_xhtml(&spans);
        assert_eq!(html, "<strong>Bold</strong> link");
    }
}
