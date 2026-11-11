//! XFA rendering: builds a painted model of the form once per document
//! (template + datasets merge, layout, paint) and serves
//!
//! * the **foreground** overlay drawn over a static form's PDF page by the
//!   page renderer (PDFium `kXFAForeground`), and
//! * **full** XFA pages for dynamic forms (PDFium `kXFAFull`), rendered
//!   from a synthesized PDF so their count may exceed the PDF's own pages.
//!
//! See `docs/xfa_static_rendering_flattening.md` for the render policy.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, OnceLock};

use serde::{Deserialize, Serialize};

use crate::cancel::CancelToken;
use crate::error::{Result, WellfriendError};
use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;
use crate::render::annotation_synthesis::Resources;
use crate::render::XfaRenderPolicy;
use crate::{ContentEngine, PdfDocument};

use super::events::FormScripts;
use super::form::{
    apply_form_packet, build_form_model_with_datasets, data_root, FormModel, Template,
};
use super::layout::layout_form;
use super::locale::LocaleSet;
use super::paint::{NamedImages, PaintScope, Painter};
use super::text::FontBook;
use super::xml::XmlNode;
use super::{load_xfa, packet_root, LoadedXfa, XfaLimits};

/// How a PDF uses XFA (PDFium `FormType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum XfaFormType {
    /// No XFA, or no usable template.
    None,
    /// Static XFA: PDF pages carry the boilerplate; XFA fields are drawn
    /// over them (`kXFAForeground`).
    Foreground,
    /// Dynamic XFA (`<dynamicRender>required</dynamicRender>`): pages come
    /// entirely from the XFA layout (`kXFAFull`).
    Full,
}

/// One painted XFA layout page (XFA space: points, y down).
#[derive(Debug)]
pub(crate) struct XfaPaintedPage {
    pub width: f64,
    pub height: f64,
    pub content_all: String,
    pub content_fields: String,
}

pub(crate) struct XfaRenderModel {
    pub form_type: XfaFormType,
    pub pages: Vec<XfaPaintedPage>,
    /// Shared resources: fonts (`XF<n>`) and shadings (`XSh<n>`).
    pub resources: PdfDictionary,
    pub unsupported: Vec<String>,
    /// `app.alert` / `xfa.host.messageBox` output of the form's scripts.
    pub script_log: Vec<String>,
    /// Script errors (first few), for diagnostics.
    pub script_errors: Vec<String>,
    /// Failed `<validate>` tests.
    pub validations: Vec<XfaValidationResult>,
    full_document: OnceLock<Option<ContentEngine>>,
}

/// A failed XFA validation test, recorded while the form was prepared for
/// rendering (no message is shown).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct XfaValidationResult {
    /// SOM expression of the field, exclusion group or subform
    /// (`xfa[0].form[0].form1[0].name[0]`).
    pub som: String,
    /// `nullTest`, `formatTest` or `scriptTest`.
    pub test: String,
    /// `error` or `warning` (the test's attribute value).
    pub severity: String,
    /// The `<validate><message>` text, else PDFium's default message.
    pub message: String,
}

/// Cached per engine; `None` for documents without a renderable XFA form.
pub(crate) type XfaRenderCache = Arc<OnceLock<Option<Arc<XfaRenderModel>>>>;

pub(crate) fn model_for(engine: &ContentEngine) -> Option<Arc<XfaRenderModel>> {
    engine
        .xfa_render_cache()
        .get_or_init(|| build_model(engine.document()).map(Arc::new))
        .clone()
}

fn xml_root<'a>(node: &'a XmlNode, name: &'a str) -> Option<&'a XmlNode> {
    if node.local_name == name {
        Some(node)
    } else {
        node.descendants(name).next()
    }
}

fn dynamic_render(config: Option<&XmlNode>) -> bool {
    config.is_some_and(|config| {
        config
            .descendants("dynamicRender")
            .any(|node| matches!(node.plain_text().to_ascii_lowercase().as_str(), "required"))
    })
}

/// The packets a render needs, borrowed from loaded XFA.
pub(crate) struct XfaSources<'a> {
    pub template: &'a XmlNode,
    pub datasets_root: Option<&'a XmlNode>,
    pub datasets: Option<&'a XmlNode>,
    pub form_type: XfaFormType,
    pub locales: LocaleSet,
    /// The saved form DOM (`<form>` packet), merged into fresh models.
    pub form: Option<&'a XmlNode>,
}

impl<'a> XfaSources<'a> {
    /// The template merged with the datasets, then with the saved `form`
    /// packet (presence, unbound values and added instances survive a
    /// save, as in PDFium).
    pub(crate) fn merged_model(&self, tpl: &Template<'a>) -> Option<FormModel<'a>> {
        let mut model =
            build_form_model_with_datasets(tpl, self.template, self.datasets, self.datasets_root)?;
        if let Some(form) = self.form {
            apply_form_packet(tpl, &mut model, form);
        }
        Some(model)
    }
}

/// Whether the document has an `/AcroForm /XFA` entry at all.
pub(crate) fn has_xfa_entry(document: &PdfDocument) -> bool {
    let reader = document.reader();
    document
        .get_catalog()
        .ok()
        .and_then(|catalog| catalog.get("AcroForm").cloned())
        .and_then(|acroform| reader.resolve(acroform).ok())
        .is_some_and(|acroform| acroform.as_dict().is_some_and(|d| d.get("XFA").is_some()))
}

pub(crate) fn xfa_sources(loaded: &LoadedXfa) -> Option<XfaSources<'_>> {
    let template = packet_root(&loaded.packets, "template")
        .and_then(|root| xml_root(root, "template"))
        .or_else(|| {
            loaded
                .packets
                .iter()
                .filter_map(|packet| packet.parsed.as_ref())
                .find_map(|parsed| xml_root(&parsed.root, "template"))
        })?;
    let datasets_root =
        packet_root(&loaded.packets, "datasets").and_then(|root| xml_root(root, "datasets"));
    let datasets = datasets_root.and_then(data_root);
    let config = packet_root(&loaded.packets, "config").and_then(|root| xml_root(root, "config"));
    let form_type = if dynamic_render(config) {
        XfaFormType::Full
    } else {
        XfaFormType::Foreground
    };
    let locale_set =
        packet_root(&loaded.packets, "localeSet").and_then(|root| xml_root(root, "localeSet"));
    let locales = LocaleSet::new(locale_set, config);
    // Only a real form packet: the `form` packet of an array, or the
    // `form` child of a whole-XDP stream.
    let form = loaded
        .packets
        .iter()
        .filter(|packet| packet.record.name.eq_ignore_ascii_case("form"))
        .find_map(|packet| packet.parsed.as_ref())
        .map(|parsed| &parsed.root)
        .and_then(|root| {
            if root.local_name == "form" {
                Some(root)
            } else {
                root.child("form")
            }
        })
        .or_else(|| {
            loaded
                .packets
                .iter()
                .filter_map(|packet| packet.parsed.as_ref())
                .find_map(|parsed| {
                    (parsed.root.local_name == "xdp")
                        .then(|| parsed.root.child("form"))
                        .flatten()
                })
        });
    Some(XfaSources {
        template,
        datasets_root,
        datasets,
        form_type,
        locales,
        form,
    })
}

fn build_model(document: &PdfDocument) -> Option<XfaRenderModel> {
    // Cheap exit for the common case: no /AcroForm /XFA.
    if !has_xfa_entry(document) {
        return None;
    }
    let limits = XfaLimits::default();
    let loaded = load_xfa(document, &limits, &CancelToken::none()).ok()?;
    let sources = xfa_sources(&loaded)?;
    let template = sources.template;
    let tpl = Template::new(template);
    let mut model = sources.merged_model(&tpl)?;
    // Form scripts (initialize, calculate, form:ready, docReady).
    let mut scripts = FormScripts::prepare(
        &tpl,
        template,
        &model,
        sources.datasets_root,
        sources.locales.clone(),
    );
    if let Some(scripts) = scripts.as_mut() {
        scripts.before_layout(&tpl, &mut model);
    }
    Some(paint_form(
        document,
        &sources,
        &tpl,
        model,
        scripts.as_mut(),
    ))
}

/// Lays out and paints a merged form model (running `layout:ready`
/// scripts, which may change the form and trigger one more layout).
pub(crate) fn paint_form<'t>(
    document: &PdfDocument,
    sources: &XfaSources<'t>,
    tpl: &Template<'t>,
    mut model: FormModel<'t>,
    mut scripts: Option<&mut FormScripts<'t>>,
) -> XfaRenderModel {
    let reader = document.reader();
    let catalog = document.get_catalog().ok();
    let acroform = catalog
        .as_ref()
        .and_then(|catalog| catalog.get("AcroForm").cloned())
        .and_then(|acroform| reader.resolve(acroform).ok())
        .and_then(|acroform| acroform.as_dict().cloned())
        .unwrap_or_else(PdfDictionary::empty);
    let dr_fonts = dr_fonts(reader, &acroform);
    let mut book = FontBook::new(Some(reader), dr_fonts);
    let mut pages = layout_form(tpl, &model, &mut book);
    let mut page_values = std::collections::HashMap::new();
    if let Some(scripts) = scripts.as_deref_mut() {
        if scripts.has_layout_ready() {
            scripts.set_layout(&pages);
            drop(pages);
            scripts.layout_ready(tpl, &mut model);
            pages = layout_form(tpl, &model, &mut book);
            scripts.set_layout(&pages);
            page_values = scripts.master_page_values(&pages);
        }
    }
    let script_report = scripts
        .as_deref()
        .map(|scripts| scripts.report())
        .unwrap_or_default();
    let named = catalog
        .as_ref()
        .map(|catalog| named_images(reader, catalog))
        .unwrap_or_default();
    let mut roots = vec![&model.root];
    roots.extend(model.page_areas.iter().flat_map(|area| area.content.iter()));
    let mut painter = Painter::new(tpl, &mut book, &named, &roots);
    painter.set_locales(sources.locales.clone());
    painter.set_page_values(page_values);
    let mut painted = Vec::with_capacity(pages.len());
    let mut shadings = BTreeMap::new();
    for (index, page) in pages.iter().enumerate() {
        let all = painter.paint_page_at(page, PaintScope::All, index);
        let fields = painter.paint_page_at(page, PaintScope::FieldsOnly, index);
        shadings.extend(all.shadings);
        shadings.extend(fields.shadings);
        painted.push(XfaPaintedPage {
            width: page.width,
            height: page.height,
            content_all: all.content,
            content_fields: fields.content,
        });
    }
    let mut unsupported = std::mem::take(&mut painter.unsupported);
    if script_report.javascript_skipped > 0 {
        unsupported.push("xfa.script.javascript_unavailable".to_string());
    }
    if script_report.failed > 0 {
        unsupported.push("xfa.script.error".to_string());
    }
    unsupported.sort();
    unsupported.dedup();
    drop(painter);
    let mut resources = Resources::default();
    book.add_to(&mut resources);
    let mut resources = resources.into_dict();
    if !shadings.is_empty() {
        let mut dict = PdfDictionary::empty();
        for (name, shading) in shadings {
            dict.insert(name, shading);
        }
        resources.insert("Shading", PdfObject::Dictionary(dict));
    }
    XfaRenderModel {
        form_type: sources.form_type,
        pages: painted,
        resources,
        unsupported,
        script_log: script_report.log,
        script_errors: script_report.errors,
        validations: script_report
            .validations
            .into_iter()
            .map(|record| XfaValidationResult {
                som: record.som,
                test: record.test.to_string(),
                severity: record.severity,
                message: record.message,
            })
            .collect(),
        full_document: OnceLock::new(),
    }
}

fn dr_fonts(reader: &PdfReader, acroform: &PdfDictionary) -> Vec<(PdfObject, PdfDictionary)> {
    let Some(dr) = acroform
        .get("DR")
        .and_then(|dr| reader.resolve(dr.clone()).ok())
    else {
        return Vec::new();
    };
    let Some(fonts) = dr
        .as_dict()
        .and_then(|dr| dr.get("Font"))
        .and_then(|fonts| reader.resolve(fonts.clone()).ok())
    else {
        return Vec::new();
    };
    let Some(fonts) = fonts.as_dict() else {
        return Vec::new();
    };
    fonts
        .iter()
        .take(256)
        .filter_map(|(_, object)| {
            let dict = reader.resolve(object.clone()).ok()?.as_dict()?.clone();
            Some((object.clone(), dict))
        })
        .collect()
}

/// `/Names /XFAImages`: images an XFA `href` may name.
fn named_images(reader: &PdfReader, catalog: &PdfDictionary) -> NamedImages {
    let mut out = HashMap::new();
    let tree = catalog
        .get("Names")
        .and_then(|names| reader.resolve(names.clone()).ok())
        .and_then(|names| names.as_dict().and_then(|d| d.get("XFAImages").cloned()))
        .and_then(|tree| reader.resolve(tree).ok());
    if let Some(tree) = tree {
        collect_name_tree(reader, &tree, &mut out, 0);
    }
    out
}

fn collect_name_tree(reader: &PdfReader, node: &PdfObject, out: &mut NamedImages, depth: usize) {
    if depth > 8 || out.len() > 512 {
        return;
    }
    let Some(dict) = node.as_dict() else {
        return;
    };
    if let Some(PdfObject::Array(names)) = dict
        .get("Names")
        .and_then(|n| reader.resolve(n.clone()).ok())
    {
        for pair in names.chunks(2) {
            let [PdfObject::String(name), value] = pair else {
                continue;
            };
            let Ok(stream) = reader.resolve(value.clone()) else {
                continue;
            };
            if let Ok(bytes) = crate::filters::decode_stream_with_limits(
                &stream,
                reader,
                &crate::filters::DecodeLimits::default(),
            ) {
                out.insert(crate::info::decode_pdf_text_string(name), bytes);
            }
        }
    }
    if let Some(PdfObject::Array(kids)) = dict
        .get("Kids")
        .and_then(|k| reader.resolve(k.clone()).ok())
    {
        for kid in kids {
            if let Ok(kid) = reader.resolve(kid) {
                collect_name_tree(reader, &kid, out, depth + 1);
            }
        }
    }
}

/// The foreground overlay of `page_number` (1-based) as a Form XObject
/// (dictionary, content, placement rectangle in PDF user space), or `None`
/// when the policy does not draw XFA on this page.
pub(crate) fn foreground_overlay(
    engine: &ContentEngine,
    page_number: usize,
    policy: XfaRenderPolicy,
) -> Option<(PdfDictionary, Vec<u8>, [f64; 4])> {
    if policy == XfaRenderPolicy::Off {
        return None;
    }
    let model = model_for(engine)?;
    let page = model.pages.get(page_number.checked_sub(1)?)?;
    let document = engine.document();
    let pdf_page = document.get_page(page_number).ok()?;
    if policy == XfaRenderPolicy::Auto
        && (model.form_type != XfaFormType::Foreground
            || page_widgets_have_appearances(document.reader(), page_annots(document, &pdf_page)))
    {
        return None;
    }
    let has_page_content = document
        .get_page_content_bytes(page_number)
        .map(|bytes| bytes.iter().any(|byte| !byte.is_ascii_whitespace()))
        .unwrap_or(false);
    let content = if model.form_type == XfaFormType::Foreground && has_page_content {
        &page.content_fields
    } else {
        &page.content_all
    };
    if content.trim().is_empty() {
        return None;
    }
    let [x0, y0, x1, y1] = pdf_page.media_box;
    let (left, top) = (x0.min(x1), y0.max(y1));
    let bbox = [x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)];
    let mut raw = format!(
        "q 1 0 0 -1 {} {} cm\n",
        crate::render::annotation_synthesis::n(left),
        crate::render::annotation_synthesis::n(top)
    )
    .into_bytes();
    raw.extend_from_slice(content.as_bytes());
    raw.extend_from_slice(b"Q\n");
    let mut dict = PdfDictionary::empty();
    dict.insert("Type", PdfObject::Name("XObject".into()));
    dict.insert("Subtype", PdfObject::Name("Form".into()));
    dict.insert(
        "BBox",
        PdfObject::Array(bbox.iter().map(|v| PdfObject::Real(*v)).collect()),
    );
    dict.insert("Resources", PdfObject::Dictionary(model.resources.clone()));
    dict.insert("Length", PdfObject::Integer(raw.len() as i64));
    Some((dict, raw, bbox))
}

/// True when the page has widget annotations and every one of them has a
/// normal appearance (`/AP /N`) — the AcroForm side is then rendered as is.
fn page_widgets_have_appearances(reader: &PdfReader, annots: Option<PdfObject>) -> bool {
    let Some(PdfObject::Array(annots)) = annots.and_then(|annots| reader.resolve(annots).ok())
    else {
        return false;
    };
    let mut widgets = 0usize;
    for annot in annots {
        let Some(annot) = reader
            .resolve(annot)
            .ok()
            .and_then(|a| a.as_dict().cloned())
        else {
            continue;
        };
        if annot.get_name("Subtype") != Some("Widget") {
            continue;
        }
        widgets += 1;
        let usable = annot
            .get("AP")
            .and_then(|ap| reader.resolve(ap.clone()).ok())
            .and_then(|ap| ap.as_dict().and_then(|ap| ap.get("N").cloned()))
            .and_then(|normal| reader.resolve(normal).ok())
            .is_some_and(|normal| {
                matches!(normal, PdfObject::Stream { .. } | PdfObject::Dictionary(_))
            });
        if !usable {
            return false;
        }
    }
    widgets > 0
}

fn page_annots(document: &PdfDocument, page: &crate::document::PdfPage) -> Option<PdfObject> {
    let page = document
        .reader()
        .get_and_resolve(page.object_number, page.generation_number)
        .ok()?;
    page.as_dict()?.get("Annots").cloned()
}

/// Whether the overlay pass may draw anything on this document's pages.
pub(crate) fn document_has_xfa_overlay(engine: &ContentEngine) -> bool {
    model_for(engine).is_some_and(|model| !model.pages.is_empty())
}

/// Serializes the XFA layout pages as a standalone PDF (fonts and other
/// referenced objects copied from `source`).
pub(crate) fn full_layout_pdf(model: &XfaRenderModel, source: &PdfReader) -> Result<Vec<u8>> {
    let mut writer = MiniWriter::default();
    // 1: catalog, 2: pages, 3: resources.
    writer.reserve(3);
    let resources = writer.import(source, &PdfObject::Dictionary(model.resources.clone()), 0);
    writer.set(3, resources);
    let mut kids = Vec::new();
    for page in &model.pages {
        let mut content = format!(
            "q 1 0 0 -1 0 {} cm\n",
            crate::render::annotation_synthesis::n(page.height)
        )
        .into_bytes();
        content.extend_from_slice(page.content_all.as_bytes());
        content.extend_from_slice(b"Q\n");
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut encoder, &content)
            .map_err(|err| WellfriendError::MalformedPdf(err.to_string()))?;
        let compressed = encoder
            .finish()
            .map_err(|err| WellfriendError::MalformedPdf(err.to_string()))?;
        let mut stream_dict = PdfDictionary::empty();
        stream_dict.insert("Filter", PdfObject::Name("FlateDecode".into()));
        let contents = writer.add(PdfObject::Stream {
            dict: stream_dict,
            raw: compressed,
        });
        let mut page_dict = PdfDictionary::empty();
        page_dict.insert("Type", PdfObject::Name("Page".into()));
        page_dict.insert("Parent", reference(2));
        page_dict.insert(
            "MediaBox",
            PdfObject::Array(vec![
                PdfObject::Integer(0),
                PdfObject::Integer(0),
                PdfObject::Real(page.width),
                PdfObject::Real(page.height),
            ]),
        );
        page_dict.insert("Resources", reference(3));
        page_dict.insert("Contents", reference(contents));
        kids.push(reference(writer.add(PdfObject::Dictionary(page_dict))));
    }
    let mut pages = PdfDictionary::empty();
    pages.insert("Type", PdfObject::Name("Pages".into()));
    pages.insert("Count", PdfObject::Integer(kids.len() as i64));
    pages.insert("Kids", PdfObject::Array(kids));
    writer.set(2, PdfObject::Dictionary(pages));
    let mut catalog = PdfDictionary::empty();
    catalog.insert("Type", PdfObject::Name("Catalog".into()));
    catalog.insert("Pages", reference(2));
    writer.set(1, PdfObject::Dictionary(catalog));
    Ok(writer.finish())
}

fn reference(number: u32) -> PdfObject {
    PdfObject::Reference {
        number,
        generation: 0,
    }
}

#[derive(Default)]
struct MiniWriter {
    objects: Vec<Option<PdfObject>>,
    imported: HashMap<(u32, u16), u32>,
}

impl MiniWriter {
    fn reserve(&mut self, count: usize) {
        self.objects.resize(self.objects.len() + count, None);
    }

    fn set(&mut self, number: u32, object: PdfObject) {
        self.objects[number as usize - 1] = Some(object);
    }

    fn add(&mut self, object: PdfObject) -> u32 {
        self.objects.push(Some(object));
        self.objects.len() as u32
    }

    /// Deep-copies `object`, giving every referenced source object a number
    /// in this document.
    fn import(&mut self, source: &PdfReader, object: &PdfObject, depth: usize) -> PdfObject {
        if depth > 64 {
            return PdfObject::Null;
        }
        match object {
            PdfObject::Reference { number, generation } => {
                if let Some(mapped) = self.imported.get(&(*number, *generation)) {
                    return reference(*mapped);
                }
                self.objects.push(None);
                let mapped = self.objects.len() as u32;
                self.imported.insert((*number, *generation), mapped);
                let resolved = source
                    .get_object(*number, *generation)
                    .unwrap_or(PdfObject::Null);
                let copied = self.import(source, &resolved, depth + 1);
                self.set(mapped, copied);
                reference(mapped)
            }
            PdfObject::Array(items) => PdfObject::Array(
                items
                    .iter()
                    .map(|item| self.import(source, item, depth + 1))
                    .collect(),
            ),
            PdfObject::Dictionary(dict) => {
                PdfObject::Dictionary(self.import_dict(source, dict, depth))
            }
            PdfObject::Stream { dict, raw } => PdfObject::Stream {
                dict: self.import_dict(source, dict, depth),
                raw: raw.clone(),
            },
            other => other.clone(),
        }
    }

    fn import_dict(
        &mut self,
        source: &PdfReader,
        dict: &PdfDictionary,
        depth: usize,
    ) -> PdfDictionary {
        let mut out = PdfDictionary::empty();
        for (key, value) in dict.iter() {
            // Never follow parent links out of the copied subgraph.
            if key == "Parent" || key == "P" {
                continue;
            }
            out.insert(key.clone(), self.import(source, value, depth + 1));
        }
        out
    }

    fn finish(self) -> Vec<u8> {
        let mut pdf = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
        let mut offsets = Vec::with_capacity(self.objects.len());
        for (index, object) in self.objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
            crate::writer::serialize_object(object.as_ref().unwrap_or(&PdfObject::Null), &mut pdf);
            pdf.extend_from_slice(b"\nendobj\n");
        }
        let xref = pdf.len();
        pdf.extend_from_slice(format!("xref\n0 {}\n", offsets.len() + 1).as_bytes());
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        for offset in offsets {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                self.objects.len() + 1
            )
            .as_bytes(),
        );
        pdf
    }
}

impl ContentEngine {
    /// How this document uses XFA ([`XfaFormType::None`] without a usable
    /// XFA template).
    pub fn xfa_form_type(&self) -> XfaFormType {
        model_for(self).map_or(XfaFormType::None, |model| model.form_type)
    }

    /// Number of XFA pages: the XFA layout's page count for dynamic
    /// ([`XfaFormType::Full`]) forms, the PDF page count for static ones,
    /// 0 without XFA. May exceed [`ContentEngine::page_count`].
    pub fn xfa_page_count(&self) -> Result<usize> {
        match model_for(self) {
            None => Ok(0),
            Some(model) if model.form_type == XfaFormType::Full => Ok(model.pages.len()),
            Some(_) => self.page_count(),
        }
    }

    /// Renders XFA page `page_number` (1-based). Dynamic forms render the
    /// XFA layout page (independent of the PDF's own, usually placeholder,
    /// pages); static forms render the PDF page with the XFA foreground
    /// forced on.
    pub fn render_xfa_page(
        &self,
        page_number: usize,
        dpi: u32,
    ) -> Result<crate::render::PixelBuffer> {
        let model = model_for(self).ok_or_else(|| {
            WellfriendError::UnsupportedFeature("document has no renderable XFA form".into())
        })?;
        if model.form_type == XfaFormType::Full {
            let count = model.pages.len();
            if page_number == 0 || page_number > count {
                return Err(WellfriendError::invalid_input(format!(
                    "XFA page {page_number} is out of range 1..={count}"
                )));
            }
            let document = model
                .full_document
                .get_or_init(|| {
                    full_layout_pdf(&model, self.document().reader())
                        .ok()
                        .and_then(|bytes| ContentEngine::open_bytes(bytes).ok())
                })
                .as_ref()
                .ok_or_else(|| {
                    WellfriendError::UnsupportedFeature("XFA layout document unavailable".into())
                })?;
            return document.render_page(page_number, dpi);
        }
        let render_mode = crate::render::RenderMode::default();
        let mut contract = self.default_render_contract(page_number, dpi, render_mode)?;
        contract.xfa = XfaRenderPolicy::Foreground;
        self.render_page_with_contract(&contract, &CancelToken::none())
    }

    /// The XFA layout as a standalone PDF: one page per XFA page with the
    /// form painted as ordinary page content (dynamic and static forms).
    pub fn xfa_layout_pdf(&self) -> Result<Vec<u8>> {
        let model = model_for(self).ok_or_else(|| {
            WellfriendError::UnsupportedFeature("document has no renderable XFA form".into())
        })?;
        full_layout_pdf(&model, self.document().reader())
    }

    /// Constructs reported while painting the XFA form (e.g. undecodable
    /// images, failed scripts).
    pub fn xfa_render_unsupported(&self) -> Vec<String> {
        model_for(self)
            .map(|model| model.unsupported.clone())
            .unwrap_or_default()
    }

    /// Messages the form's scripts showed while the XFA form was prepared
    /// for rendering (`app.alert`, `xfa.host.messageBox`), in order.
    pub fn xfa_script_log(&self) -> Vec<String> {
        model_for(self)
            .map(|model| model.script_log.clone())
            .unwrap_or_default()
    }

    /// The first errors raised by the form's scripts (syntax errors,
    /// unknown accessors, exhausted budgets), for diagnostics.
    pub fn xfa_script_errors(&self) -> Vec<String> {
        model_for(self)
            .map(|model| model.script_errors.clone())
            .unwrap_or_default()
    }

    /// Failed `<validate>` tests (`nullTest`, `formatTest`, validate
    /// scripts) of the XFA form after its scripts ran, in document order.
    /// Validation runs non-interactively, as PDFium's on document load; it
    /// never changes what is drawn.
    pub fn xfa_validation_results(&self) -> Vec<XfaValidationResult> {
        model_for(self)
            .map(|model| model.validations.clone())
            .unwrap_or_default()
    }
}
