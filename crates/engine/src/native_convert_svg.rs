//! SVG to vector PDF via `usvg` (parse/normalise) and `svg2pdf` (PDF form
//! XObject). Only `data:` images are resolved; external files and URLs are
//! never read, and scripts are ignored by `usvg`. Text uses the bundled
//! Liberation and DejaVu faces only; no system fonts are loaded.
use super::*;
use pdf_writer::{Content, Finish, Name, Pdf, Rect, Ref, TextStr};
use std::collections::HashMap;
use svg2pdf::usvg;

/// Largest page edge PDF viewers reliably support (200 inches).
const MAX_PAGE_EDGE: f64 = 14_400.0;

/// Page layout for [`svg_to_pdf`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SvgPageMode {
    /// One page exactly the SVG's size (CSS pixels at 96 per inch).
    #[default]
    Natural,
    /// Scale the drawing to fit `page_size` inside the margins, centred.
    Fit,
}

/// Options for [`svg_to_pdf`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SvgToPdfOptions {
    pub page: SvgPageMode,
    /// Used with `page: "fit"`.
    pub page_size: NativePageSize,
    /// With `page: "fit"`, use landscape when the drawing is wider than tall.
    pub auto_orient: bool,
    /// With `page: "fit"`, force `portrait` or `landscape` (overrides
    /// `auto_orient`).
    pub orientation: Option<NativeOrientation>,
    /// With `page: "fit"`, margins around the drawing.
    pub margins: NativeMargins,
    pub title: Option<String>,
    pub author: Option<String>,
}

impl Default for SvgToPdfOptions {
    fn default() -> Self {
        Self {
            page: SvgPageMode::Natural,
            page_size: NativePageSize::A4,
            auto_orient: true,
            orientation: None,
            margins: NativeMargins::All(36.0),
            title: None,
            author: None,
        }
    }
}

pub fn svg_to_pdf(input: &[u8], options: &SvgToPdfOptions) -> Result<Vec<u8>> {
    Ok(svg_to_pdf_with_report(input, options)?.pdf)
}

fn usvg_options() -> usvg::Options<'static> {
    let mut options = usvg::Options {
        resources_dir: None,
        font_family: "Liberation Sans".to_string(),
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: usvg::ImageHrefResolver::default_data_resolver(),
            // Never read files or URLs named by `href`.
            resolve_string: Box::new(|_, _| None),
        },
        ..usvg::Options::default()
    };
    let database = options.fontdb_mut();
    for name in [
        "Helvetica",
        "Helvetica-Bold",
        "Helvetica-Oblique",
        "Helvetica-BoldOblique",
        "Times-Roman",
        "Times-Bold",
        "Times-Italic",
        "Times-BoldItalic",
        "Courier",
        "Courier-Bold",
        "Courier-Oblique",
        "Courier-BoldOblique",
        "Symbol",
    ] {
        if let Some(bytes) = get_fallback_font(name) {
            database.load_font_data(bytes.to_vec());
        }
    }
    database.set_sans_serif_family("Liberation Sans");
    database.set_serif_family("Liberation Serif");
    database.set_monospace_family("Liberation Mono");
    database.set_cursive_family("Liberation Serif");
    database.set_fantasy_family("Liberation Sans");
    options
}

pub fn svg_to_pdf_with_report(
    input: &[u8],
    options: &SvgToPdfOptions,
) -> Result<NativeConversionOutput> {
    check_input_size(input)?;
    if input.iter().all(u8::is_ascii_whitespace) {
        return Err(input_error(codes::EMPTY_INPUT, "SVG input is empty"));
    }
    let tree = usvg::Tree::from_data(input, &usvg_options())
        .map_err(|error| input_error(codes::SVG_PARSE_ERROR, error))?;
    let svg_width = f64::from(tree.size().width()) * 0.75;
    let svg_height = f64::from(tree.size().height()) * 0.75;
    if !(svg_width.is_finite() && svg_height.is_finite() && svg_width > 0.0 && svg_height > 0.0) {
        return Err(input_error(
            codes::SVG_PARSE_ERROR,
            "SVG has no drawable size",
        ));
    }
    let mut report = NativeConversionReport::default();
    let (page_width, page_height, draw) = match options.page {
        SvgPageMode::Natural => {
            let scale = (MAX_PAGE_EDGE / svg_width)
                .min(MAX_PAGE_EDGE / svg_height)
                .min(1.0);
            if scale < 1.0 {
                report
                    .warnings
                    .push("SVG exceeds 200 inches; scaled down to the PDF page limit".into());
            }
            let (width, height) = (svg_width * scale, svg_height * scale);
            (width, height, [width, height, 0.0, 0.0])
        }
        SvgPageMode::Fit => {
            let page = oriented(
                options.page_size.page_size(),
                options.auto_orient && svg_width > svg_height,
                options.orientation,
            );
            let margins = options.margins.resolve(page)?;
            let box_width = page.width - margins.left - margins.right;
            let box_height = page.height - margins.top - margins.bottom;
            let scale = (box_width / svg_width).min(box_height / svg_height);
            let (width, height) = (svg_width * scale, svg_height * scale);
            let x = margins.left + (box_width - width) / 2.0;
            let y = margins.bottom + (box_height - height) / 2.0;
            (page.width, page.height, [width, height, x, y])
        }
    };

    let (chunk, svg_ref) = svg2pdf::to_chunk(&tree, svg2pdf::ConversionOptions::default())
        .map_err(|error| input_error(codes::SVG_CONVERT_ERROR, error))?;
    let mut alloc = Ref::new(1);
    let catalog_id = alloc.bump();
    let page_tree_id = alloc.bump();
    let page_id = alloc.bump();
    let content_id = alloc.bump();
    let info_id = alloc.bump();
    let mut map = HashMap::new();
    let chunk = chunk.renumber(|old| *map.entry(old).or_insert_with(|| alloc.bump()));
    let svg_id = *map
        .get(&svg_ref)
        .ok_or_else(|| input_error(codes::SVG_CONVERT_ERROR, "missing SVG form XObject"))?;
    let svg_name = Name(b"WfSvg");

    let mut pdf = Pdf::new();
    pdf.catalog(catalog_id).pages(page_tree_id);
    pdf.pages(page_tree_id).kids([page_id]).count(1);
    let mut page = pdf.page(page_id);
    page.media_box(Rect::new(0.0, 0.0, page_width as f32, page_height as f32));
    page.parent(page_tree_id);
    page.contents(content_id);
    page.resources().x_objects().pair(svg_name, svg_id);
    page.finish();

    let [width, height, x, y] = draw;
    let mut content = Content::new();
    content.save_state();
    content.transform([width as f32, 0.0, 0.0, height as f32, x as f32, y as f32]);
    content.x_object(svg_name);
    content.restore_state();
    pdf.stream(content_id, &content.finish());

    let mut info = pdf.document_info(info_id);
    if let Some(title) = options
        .title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        info.title(TextStr(title));
    }
    if let Some(author) = options
        .author
        .as_deref()
        .map(str::trim)
        .filter(|a| !a.is_empty())
    {
        info.author(TextStr(author));
    }
    info.creator(TextStr("Wellfriend PDF SDK svg-to-pdf"));
    info.producer(TextStr("Wellfriend PDF SDK (svg2pdf)"));
    info.finish();
    pdf.extend(&chunk);

    let bytes = pdf.finish();
    report.pages = 1;
    report.bytes = bytes.len();
    Ok(NativeConversionOutput { pdf: bytes, report })
}
