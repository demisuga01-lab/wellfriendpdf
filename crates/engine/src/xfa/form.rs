//! XFA form model: the template merged with the datasets packet into a tree
//! of form nodes (PDFium's form DOM, much simplified).
//!
//! Merge rules follow the XFA "data merge" defaults: a named container binds
//! to the first unconsumed data child of its scope with the same name
//! (`match="once"`), `global` searches the whole data tree, `dataRef`
//! resolves a SOM-like path, and `none` / unnamed containers do not bind.
//! Repeating subforms (`<occur>`) get one instance per matching data group,
//! bounded by `min`/`max`, or `initial` instances without data.

use std::collections::{HashMap, HashSet};

use super::parse_measurement;
use super::style::{Presence, Rgb};
use super::xml::XmlNode;

/// Hard cap on generated form nodes, guarding against hostile `occur`/data.
const MAX_FORM_NODES: usize = 200_000;

/// Template property access with `use="#id"` prototype inheritance.
pub(crate) struct Template<'t> {
    ids: HashMap<&'t str, &'t XmlNode>,
}

impl<'t> Template<'t> {
    pub(crate) fn new(root: &'t XmlNode) -> Self {
        let mut ids = HashMap::new();
        let mut stack = vec![(root, 0usize)];
        while let Some((node, depth)) = stack.pop() {
            if let Some(id) = node.attr("id") {
                ids.entry(id).or_insert(node);
            }
            if depth < 256 {
                stack.extend(node.children.iter().map(|child| (child, depth + 1)));
            }
        }
        Self { ids }
    }

    fn proto(&self, node: &'t XmlNode) -> Option<&'t XmlNode> {
        let reference = node.attr("use").or_else(|| node.attr("usehref"))?;
        let id = reference.rsplit_once('#').map(|(_, id)| id)?;
        self.ids
            .get(id)
            .copied()
            .filter(|proto| !std::ptr::eq(*proto, node))
    }

    /// First child element `name` of `node` or of its prototypes.
    pub(crate) fn child(&self, node: &'t XmlNode, name: &str) -> Option<&'t XmlNode> {
        let mut current = Some(node);
        for _ in 0..8 {
            let candidate = current?;
            if let Some(child) = candidate.child(name) {
                return Some(child);
            }
            current = self.proto(candidate);
        }
        None
    }

    /// Template element by `id`.
    pub(crate) fn by_id(&self, id: &str) -> Option<&'t XmlNode> {
        self.ids.get(id).copied()
    }

    /// Attribute `name` of `node` or of its prototypes.
    pub(crate) fn attr(&self, node: &'t XmlNode, name: &str) -> Option<&'t str> {
        let mut current = Some(node);
        for _ in 0..8 {
            let candidate = current?;
            if let Some(value) = candidate.attr(name) {
                return Some(value);
            }
            current = self.proto(candidate);
        }
        None
    }

    /// Measurement attribute in points.
    pub(crate) fn measure(&self, node: &'t XmlNode, name: &str) -> Option<f64> {
        self.attr(node, name)
            .and_then(|value| parse_measurement(value).ok())
            .filter(|value| value.is_finite())
    }

    /// Path of child elements, e.g. `["ui", "textEdit"]`.
    pub(crate) fn path(&self, node: &'t XmlNode, names: &[&str]) -> Option<&'t XmlNode> {
        let mut current = node;
        for name in names {
            current = self.child(current, name)?;
        }
        Some(current)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NodeKind {
    Subform,
    Area,
    ExclGroup,
    Field,
    Draw,
}

/// An image value: `href` or inline base64 content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ImageValue {
    pub content_type: String,
    pub data: String,
    pub href: Option<String>,
}

#[derive(Debug)]
pub(crate) struct FormNode<'t> {
    pub kind: NodeKind,
    pub tmpl: &'t XmlNode,
    /// Merged raw (canonical) value for fields and exclusion groups.
    pub value: Option<String>,
    /// Rich-text (`exData` XHTML) template value, when the value is rich.
    pub rich: Option<&'t XmlNode>,
    pub image: Option<ImageValue>,
    /// Choice-list items bound from data (`<bindItems>`) or set by
    /// scripts: (display, save).
    pub items: Option<Vec<(String, String)>>,
    pub children: Vec<FormNode<'t>>,
    /// Effective locale name (the nearest `locale` attribute).
    pub locale: Option<&'t str>,
    /// Script overrides of template properties.
    pub presence: Option<Presence>,
    pub caption: Option<String>,
    pub caption_presence: Option<Presence>,
    pub fill_color: Option<Rgb>,
    pub border_color: Option<Rgb>,
    pub font_color: Option<Rgb>,
    /// The datasets element the node was merged with (fields, exclusion
    /// groups, subform instances, the root's record).
    pub data: Option<&'t XmlNode>,
}

impl<'t> FormNode<'t> {
    pub(crate) fn new(kind: NodeKind, tmpl: &'t XmlNode) -> Self {
        Self {
            kind,
            tmpl,
            value: None,
            rich: None,
            image: None,
            items: None,
            children: Vec::new(),
            locale: None,
            presence: None,
            caption: None,
            caption_presence: None,
            fill_color: None,
            border_color: None,
            font_color: None,
            data: None,
        }
    }

    /// Presence: the script override, else the template's.
    pub(crate) fn presence_in(&self, tpl: &Template<'t>) -> Presence {
        self.presence
            .unwrap_or_else(|| super::style::presence(tpl, self.tmpl))
    }

    #[cfg(test)]
    pub(crate) fn name(&self) -> &'t str {
        self.tmpl.attr("name").unwrap_or("")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PagePosition {
    Any,
    First,
    Last,
    Rest,
    Only,
}

#[derive(Debug)]
pub(crate) struct PageAreaModel<'t> {
    /// The `<pageArea>` element (the template root for the default page).
    pub tmpl: &'t XmlNode,
    pub name: &'t str,
    pub id: Option<&'t str>,
    pub width: f64,
    pub height: f64,
    /// Content areas in page coordinates (x, y, w, h), template order.
    pub content_areas: Vec<[f64; 4]>,
    /// (id, name) of each content area.
    pub content_ids: Vec<(Option<&'t str>, &'t str)>,
    pub content: Vec<FormNode<'t>>,
    pub max_occur: Option<usize>,
    pub position: PagePosition,
}

#[derive(Debug)]
pub(crate) struct FormModel<'t> {
    pub root: FormNode<'t>,
    pub page_areas: Vec<PageAreaModel<'t>>,
    /// `orderedOccurrence` (false) or `simplexPaginated`/`duplexPaginated`.
    pub paginated: bool,
    /// Leader/trailer subforms (`<overflow>`, `<bookend>`, break
    /// `leader`/`trailer`) by their reference text, instantiated once
    /// with template defaults.
    pub extras: HashMap<String, FormNode<'t>>,
}

/// The template's root subform: `template > subform`.
pub(crate) fn template_root(template: &XmlNode) -> Option<&XmlNode> {
    if template.local_name == "subform" {
        return Some(template);
    }
    template
        .children
        .iter()
        .find(|child| child.local_name == "subform")
}

/// The `<xfa:data>` element of a datasets packet.
pub(crate) fn data_root(datasets: &XmlNode) -> Option<&XmlNode> {
    if datasets.local_name == "data" {
        return Some(datasets);
    }
    datasets
        .children
        .iter()
        .find(|child| child.local_name == "data")
}

#[cfg(test)]
pub(crate) fn build_form_model<'t>(
    tpl: &Template<'t>,
    template: &'t XmlNode,
    data: Option<&'t XmlNode>,
) -> Option<FormModel<'t>> {
    build_form_model_with_datasets(tpl, template, data, None)
}

/// Merges `template` with `data` (`<xfa:data>`); `datasets` (the whole
/// packet) serves `<bindItems>` references such as
/// `xfa.datasets.LOV.Countries.Country[*]`.
pub(crate) fn build_form_model_with_datasets<'t>(
    tpl: &Template<'t>,
    template: &'t XmlNode,
    data: Option<&'t XmlNode>,
    datasets: Option<&'t XmlNode>,
) -> Option<FormModel<'t>> {
    let root_tmpl = template_root(template)?;
    let mut merger = Merger {
        tpl,
        consumed: HashSet::new(),
        data,
        datasets,
        nodes: 0,
    };
    let root_name = root_tmpl.attr("name").unwrap_or("");
    let record = data.and_then(|data| {
        let elements: Vec<&XmlNode> = data.children.iter().collect();
        elements
            .iter()
            .copied()
            .find(|child| !root_name.is_empty() && child.local_name == root_name)
            .or_else(|| elements.first().copied().filter(|_| elements.len() == 1))
    });
    let mut root = FormNode::new(NodeKind::Subform, root_tmpl);
    root.data = record;
    root.children = merger.merge_children(root_tmpl, record);

    let mut page_areas = Vec::new();
    let mut paginated = false;
    if let Some(page_set) = root_tmpl
        .children
        .iter()
        .find(|child| child.local_name == "pageSet")
    {
        paginated = matches!(
            tpl.attr(page_set, "relation"),
            Some("simplexPaginated" | "duplexPaginated")
        );
        collect_page_areas(&mut merger, page_set, &mut page_areas, 0);
    }
    if page_areas.is_empty() {
        page_areas.push(PageAreaModel {
            tmpl: root_tmpl,
            name: "",
            id: None,
            width: 612.0,
            height: 792.0,
            content_areas: vec![[0.0, 0.0, 612.0, 792.0]],
            content_ids: vec![(None, "")],
            content: Vec::new(),
            max_occur: None,
            position: PagePosition::Any,
        });
    }
    let extras = leader_trailer_instances(tpl, template);
    let mut model = FormModel {
        root,
        page_areas,
        paginated,
        extras,
    };
    let ambient = None;
    assign_locales(&mut model.root, ambient);
    let root_locale = model.root.locale;
    for extra in model.extras.values_mut() {
        assign_locales(extra, root_locale);
    }
    for area in &mut model.page_areas {
        let locale = area.tmpl.attr("locale").or(root_locale);
        for node in &mut area.content {
            assign_locales(node, locale);
        }
    }
    Some(model)
}

/// The template subform a leader/trailer reference names: `#id`, or a
/// (SOM-like) name whose last segment names a subform.
pub(crate) fn resolve_template_ref<'t>(
    tpl: &Template<'t>,
    template: &'t XmlNode,
    reference: &str,
) -> Option<&'t XmlNode> {
    let reference = reference.trim();
    if reference.is_empty() {
        return None;
    }
    if let Some(id) = reference.strip_prefix('#') {
        return tpl.by_id(id).filter(|node| node.local_name == "subform");
    }
    let name = reference
        .rsplit('.')
        .next()
        .unwrap_or(reference)
        .split('[')
        .next()
        .unwrap_or("");
    if name.is_empty() {
        return None;
    }
    template
        .descendants("subform")
        .find(|node| node.attr("name") == Some(name))
}

/// Instances of every subform referenced as a leader or trailer.
fn leader_trailer_instances<'t>(
    tpl: &Template<'t>,
    template: &'t XmlNode,
) -> HashMap<String, FormNode<'t>> {
    const ATTRS: [&str; 8] = [
        "leader",
        "trailer",
        "overflowLeader",
        "overflowTrailer",
        "beforeLeader",
        "beforeTrailer",
        "afterLeader",
        "afterTrailer",
    ];
    let mut out = HashMap::new();
    let mut stack = vec![template];
    let mut seen = 0usize;
    while let Some(node) = stack.pop() {
        seen += 1;
        if seen > 200_000 || out.len() >= 256 {
            break;
        }
        if matches!(
            node.local_name.as_str(),
            "overflow" | "bookend" | "breakBefore" | "breakAfter" | "break"
        ) {
            for attr in ATTRS {
                let Some(reference) = node.attr(attr).map(str::trim).filter(|r| !r.is_empty())
                else {
                    continue;
                };
                if out.contains_key(reference) {
                    continue;
                }
                if let Some(tmpl) = resolve_template_ref(tpl, template, reference) {
                    out.insert(reference.to_string(), instance_prototype(tpl, tmpl));
                }
            }
        }
        stack.extend(node.children.iter());
    }
    out
}

/// Sets each node's effective locale: its own `locale` attribute or the
/// nearest ancestor's.
pub(crate) fn assign_locales<'t>(node: &mut FormNode<'t>, inherited: Option<&'t str>) {
    let own = node
        .tmpl
        .attr("locale")
        .filter(|locale| !locale.trim().is_empty() && *locale != "ambient");
    node.locale = own.or(inherited);
    let locale = node.locale;
    for child in &mut node.children {
        assign_locales(child, locale);
    }
}

/// One fresh instance of the repeatable subform `tmpl` with template
/// defaults and no data (instance-manager `addInstance`).
pub(crate) fn instance_prototype<'t>(tpl: &Template<'t>, tmpl: &'t XmlNode) -> FormNode<'t> {
    let mut merger = Merger {
        tpl,
        consumed: HashSet::new(),
        data: None,
        datasets: None,
        nodes: 0,
    };
    let mut node = FormNode::new(NodeKind::Subform, tmpl);
    node.children = merger.merge_children(tmpl, None);
    node
}

/// Namespace of the `form` packets a session saves.
pub(crate) const XFA_FORM_NS: &str = "http://www.xfa.org/schema/xfa-form/2.8/";

fn kind_element(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Subform => "subform",
        NodeKind::Area => "area",
        NodeKind::ExclGroup => "exclGroup",
        NodeKind::Field => "field",
        NodeKind::Draw => "draw",
    }
}

/// The presence a node shows: its override, else the template's.
pub(crate) fn presence_text<'t>(tpl: &Template<'t>, node: &FormNode<'t>) -> &'t str {
    let template = tpl.attr(node.tmpl, "presence").unwrap_or("visible");
    match node.presence {
        None => template,
        Some(Presence::Visible) => "visible",
        Some(Presence::Invisible) => "invisible",
        Some(Presence::Hidden) if matches!(template, "hidden" | "inactive") => template,
        Some(Presence::Hidden) => "hidden",
    }
}

/// The form DOM of `model` as a `form` packet (PDFium saves the form packet
/// next to `datasets`, `CXFA_FFDoc::SavePackage`): the merged containers of
/// the root subform in document order, with their names, presence and the
/// values of fields (repeated subform instances appear once each). Rich
/// text and image values are left to the template and data.
pub(crate) fn form_packet_xml<'t>(tpl: &Template<'t>, model: &FormModel<'t>) -> String {
    let mut out = String::new();
    out.push_str("<form xmlns=\"");
    out.push_str(XFA_FORM_NS);
    out.push_str("\">");
    write_form_node(tpl, &model.root, &mut out, 0);
    out.push_str("</form>");
    out
}

fn write_form_node<'t>(tpl: &Template<'t>, node: &FormNode<'t>, out: &mut String, depth: usize) {
    use super::xml::escape_xml;
    if depth > 256 {
        return;
    }
    let element = kind_element(node.kind);
    out.push('<');
    out.push_str(element);
    if let Some(name) = node.tmpl.attr("name").filter(|name| !name.is_empty()) {
        out.push_str(" name=\"");
        escape_xml(name, out);
        out.push('"');
    }
    out.push_str(" presence=\"");
    out.push_str(presence_text(tpl, node));
    out.push_str("\">");
    if node.kind == NodeKind::Field && node.rich.is_none() && node.image.is_none() {
        let content = tpl
            .child(node.tmpl, "value")
            .and_then(|value| value.children.first())
            .map(|content| content.local_name.as_str())
            .filter(|name| !matches!(*name, "exData" | "image"))
            .unwrap_or("text");
        out.push_str("<value><");
        out.push_str(content);
        out.push('>');
        if let Some(value) = &node.value {
            escape_xml(value, out);
        }
        out.push_str("</");
        out.push_str(content);
        out.push_str("></value>");
    }
    for child in &node.children {
        write_form_node(tpl, child, out, depth + 1);
    }
    out.push_str("</");
    out.push_str(element);
    out.push('>');
}

/// Merges a saved `form` packet into a freshly merged model, as PDFium's
/// data merge reuses the form DOM it loaded (`XFA_NodeMerge_CloneOrMerge
/// Container`): saved containers match the model's children by element,
/// name and position; their `presence` applies; fields not bound to data
/// take the saved value (bound fields keep the data's); a repeatable
/// subform saved with more instances than the merge produced gets the
/// missing instances (bounded by `occur max`). Returns whether anything
/// changed.
pub(crate) fn apply_form_packet<'t>(
    tpl: &Template<'t>,
    model: &mut FormModel<'t>,
    form: &XmlNode,
) -> bool {
    let root_name = model.root.tmpl.attr("name").unwrap_or("");
    let Some(saved) = form
        .children
        .iter()
        .find(|child| child.local_name == "subform")
        .filter(|saved| saved.attr("name").unwrap_or("") == root_name)
    else {
        return false;
    };
    let mut budget = MAX_FORM_NODES;
    let changed = apply_saved_node(tpl, &mut model.root, saved, &mut budget, 0);
    if changed {
        assign_locales(&mut model.root, None);
    }
    changed
}

fn apply_saved_node<'t>(
    tpl: &Template<'t>,
    node: &mut FormNode<'t>,
    saved: &XmlNode,
    budget: &mut usize,
    depth: usize,
) -> bool {
    if depth > 256 || *budget == 0 {
        return false;
    }
    *budget -= 1;
    let mut changed = false;
    if let Some(presence) = saved.attr("presence") {
        if presence != presence_text(tpl, node) {
            node.presence = Some(match presence {
                "invisible" => Presence::Invisible,
                "hidden" | "inactive" => Presence::Hidden,
                _ => Presence::Visible,
            });
            changed = true;
        }
    }
    if node.kind == NodeKind::Field
        && node.data.is_none()
        && node.rich.is_none()
        && node.image.is_none()
    {
        if let Some(content) = saved
            .child("value")
            .and_then(|value| value.children.first())
        {
            let value = (!content.text.is_empty()).then(|| content.text.clone());
            if node.value != value {
                node.value = value;
                changed = true;
            }
        }
    }
    changed |= add_saved_instances(tpl, node, saved, budget);
    let mut seen: HashMap<(&'static str, &'t str), usize> = HashMap::new();
    for child in &mut node.children {
        let element = kind_element(child.kind);
        let name = child.tmpl.attr("name").unwrap_or("");
        let index = seen.entry((element, name)).or_insert(0);
        let matched = saved
            .children
            .iter()
            .filter(|c| c.local_name == element && c.attr("name").unwrap_or("") == name)
            .nth(*index);
        *index += 1;
        if let Some(matched) = matched {
            changed |= apply_saved_node(tpl, child, matched, budget, depth + 1);
        }
    }
    changed
}

/// Appends instances of named repeatable subforms the saved node has more
/// of than `node` (instances a script added before the save).
fn add_saved_instances<'t>(
    tpl: &Template<'t>,
    node: &mut FormNode<'t>,
    saved: &XmlNode,
    budget: &mut usize,
) -> bool {
    let mut changed = false;
    let mut index = 0;
    while index < node.children.len() {
        let tmpl = node.children[index].tmpl;
        let Some(occur) = (node.children[index].kind == NodeKind::Subform)
            .then(|| tpl.child(tmpl, "occur"))
            .flatten()
        else {
            index += 1;
            continue;
        };
        let name = tmpl.attr("name").unwrap_or("");
        // The run of instances of this template.
        let mut end = index;
        while end < node.children.len() && std::ptr::eq(node.children[end].tmpl, tmpl) {
            end += 1;
        }
        if name.is_empty() {
            index = end;
            continue;
        }
        let have = end - index;
        let wanted = saved
            .children
            .iter()
            .filter(|c| c.local_name == "subform" && c.attr("name") == Some(name))
            .count();
        let max = match tpl.attr(occur, "max").map(str::trim) {
            Some("-1" | "*") => usize::MAX,
            Some(max) => max.parse::<usize>().unwrap_or(1),
            None => tpl
                .attr(occur, "min")
                .and_then(|min| min.trim().parse::<usize>().ok())
                .unwrap_or(1)
                .max(1),
        };
        let wanted = wanted.min(max).min(have + *budget);
        let mut at = end;
        for _ in have..wanted {
            let instance = instance_prototype(tpl, tmpl);
            *budget = budget.saturating_sub(1);
            node.children.insert(at, instance);
            at += 1;
            changed = true;
        }
        index = at;
    }
    changed
}

/// A node with its template defaults (value, rich text, image, items) and
/// no children.
pub(crate) fn fresh_node<'t>(
    tpl: &Template<'t>,
    kind: NodeKind,
    tmpl: &'t XmlNode,
) -> FormNode<'t> {
    let mut node = FormNode::new(kind, tmpl);
    if matches!(kind, NodeKind::Field | NodeKind::Draw) {
        let (value, rich, image) = template_value(tpl, tmpl);
        node.value = value;
        node.rich = rich;
        node.image = image;
    }
    node
}

fn collect_page_areas<'t>(
    merger: &mut Merger<'_, 't>,
    page_set: &'t XmlNode,
    out: &mut Vec<PageAreaModel<'t>>,
    depth: usize,
) {
    if depth > 8 {
        return;
    }
    for child in &page_set.children {
        match child.local_name.as_str() {
            "pageSet" => collect_page_areas(merger, child, out, depth + 1),
            "pageArea" => out.push(page_area_model(merger, child)),
            _ => {}
        }
    }
}

fn page_area_model<'t>(merger: &mut Merger<'_, 't>, area: &'t XmlNode) -> PageAreaModel<'t> {
    let tpl = merger.tpl;
    let (mut width, mut height) = (612.0, 792.0);
    if let Some(medium) = tpl.child(area, "medium") {
        if let Some((w, h)) = tpl.attr(medium, "stock").and_then(stock_size) {
            width = w;
            height = h;
        }
        if let Some(short) = tpl.measure(medium, "short") {
            width = short;
        }
        if let Some(long) = tpl.measure(medium, "long") {
            height = long;
        }
        if tpl.attr(medium, "orientation") == Some("landscape") {
            std::mem::swap(&mut width, &mut height);
        }
    }
    let width = width.max(1.0);
    let height = height.max(1.0);
    let mut content_areas = Vec::new();
    let mut content_ids = Vec::new();
    for child in area
        .children
        .iter()
        .filter(|child| child.local_name == "contentArea")
    {
        content_ids.push((child.attr("id"), child.attr("name").unwrap_or("")));
        let x = tpl.measure(child, "x").unwrap_or(0.0);
        let y = tpl.measure(child, "y").unwrap_or(0.0);
        let w = tpl.measure(child, "w").unwrap_or(width - x).max(1.0);
        let h = tpl.measure(child, "h").unwrap_or(height - y).max(1.0);
        content_areas.push([x, y, w, h]);
    }
    let max_occur = tpl.child(area, "occur").and_then(|occur| {
        tpl.attr(occur, "max")
            .and_then(|max| max.trim().parse::<i64>().ok())
            .and_then(|max| usize::try_from(max).ok())
    });
    let position = match tpl.attr(area, "pagePosition") {
        Some("first") => PagePosition::First,
        Some("last") => PagePosition::Last,
        Some("rest") => PagePosition::Rest,
        Some("only") => PagePosition::Only,
        _ => PagePosition::Any,
    };
    let content = merger.merge_children(area, None);
    PageAreaModel {
        tmpl: area,
        name: area.attr("name").unwrap_or(""),
        id: area.attr("id"),
        width,
        height,
        content_areas,
        content_ids,
        content,
        max_occur,
        position,
    }
}

fn stock_size(stock: &str) -> Option<(f64, f64)> {
    let mm = |w: f64, h: f64| (w * 72.0 / 25.4, h * 72.0 / 25.4);
    Some(match stock.to_ascii_lowercase().as_str() {
        "letter" | "default" => (612.0, 792.0),
        "legal" => (612.0, 1008.0),
        "ledger" | "tabloid" => (792.0, 1224.0),
        "executive" => (522.0, 756.0),
        "a3" => mm(297.0, 420.0),
        "a4" => mm(210.0, 297.0),
        "a5" => mm(148.0, 210.0),
        "b4" => mm(250.0, 353.0),
        "b5" => mm(176.0, 250.0),
        _ => return None,
    })
}

struct Merger<'a, 't> {
    tpl: &'a Template<'t>,
    consumed: HashSet<usize>,
    data: Option<&'t XmlNode>,
    datasets: Option<&'t XmlNode>,
    nodes: usize,
}

#[derive(Clone, Copy)]
struct Occur {
    min: usize,
    max: Option<usize>,
    initial: usize,
}

impl<'t> Merger<'_, 't> {
    fn budget(&mut self) -> bool {
        self.nodes += 1;
        self.nodes <= MAX_FORM_NODES
    }

    fn merge_children(
        &mut self,
        tmpl: &'t XmlNode,
        scope: Option<&'t XmlNode>,
    ) -> Vec<FormNode<'t>> {
        let mut out = Vec::new();
        for child in &tmpl.children {
            if !self.budget() {
                break;
            }
            match child.local_name.as_str() {
                "subform" => self.merge_subform(child, scope, &mut out),
                "subformSet" => out.extend(self.merge_children(child, scope)),
                "area" => {
                    let mut node = FormNode::new(NodeKind::Area, child);
                    node.children = self.merge_children(child, scope);
                    out.push(node);
                }
                "field" => out.push(self.merge_field(child, scope)),
                "exclGroup" => out.push(self.merge_excl_group(child, scope)),
                "draw" => {
                    let mut node = FormNode::new(NodeKind::Draw, child);
                    let (value, rich, image) = template_value(self.tpl, child);
                    node.value = value;
                    node.rich = rich;
                    node.image = image;
                    out.push(node);
                }
                _ => {}
            }
        }
        out
    }

    fn occur(&self, node: &'t XmlNode) -> Occur {
        let Some(occur) = self.tpl.child(node, "occur") else {
            return Occur {
                min: 1,
                max: Some(1),
                initial: 1,
            };
        };
        let int = |name: &str| {
            self.tpl
                .attr(occur, name)
                .and_then(|value| value.trim().parse::<i64>().ok())
        };
        let min = int("min").unwrap_or(1).clamp(0, 10_000) as usize;
        let max = match int("max") {
            Some(max) if max < 0 => None,
            Some(max) => Some((max as usize).max(min)),
            None => Some(min.max(1)),
        };
        let initial = int("initial").map_or(min, |v| v.clamp(0, 10_000) as usize);
        Occur { min, max, initial }
    }

    fn binding(&self, node: &'t XmlNode) -> (&'t str, Option<&'t str>) {
        binding_mode(self.tpl, node)
    }

    fn merge_subform(
        &mut self,
        tmpl: &'t XmlNode,
        scope: Option<&'t XmlNode>,
        out: &mut Vec<FormNode<'t>>,
    ) {
        let name = tmpl.attr("name").unwrap_or("");
        let occur = self.occur(tmpl);
        let (mode, reference) = self.binding(tmpl);
        let max = occur.max.unwrap_or(usize::MAX);
        let mut transparent = false;
        let data_nodes: Vec<&'t XmlNode> = match mode {
            "none" => {
                transparent = true;
                Vec::new()
            }
            "global" => self.global(name).into_iter().take(max).collect(),
            "dataRef" => reference
                .map(|reference| self.resolve_ref(reference, scope))
                .unwrap_or_default()
                .into_iter()
                .take(max)
                .collect(),
            _ => {
                let found: Vec<&'t XmlNode> = scope
                    .map(|scope| {
                        scope
                            .children
                            .iter()
                            .filter(|child| {
                                child.local_name == name && !self.consumed.contains(&key(child))
                            })
                            .take(max)
                            .collect()
                    })
                    .unwrap_or_default();
                for node in &found {
                    self.consumed.insert(key(node));
                }
                found
            }
        };
        let count = if data_nodes.is_empty() {
            occur.initial.max(occur.min).min(max)
        } else {
            data_nodes.len().max(occur.min).min(max)
        };
        for index in 0..count.min(10_000) {
            if !self.budget() {
                return;
            }
            let instance_scope = match data_nodes.get(index) {
                Some(node) => Some(*node),
                None if transparent => scope,
                None => None,
            };
            let mut node = FormNode::new(NodeKind::Subform, tmpl);
            node.data = data_nodes.get(index).copied();
            node.children = self.merge_children(tmpl, instance_scope);
            out.push(node);
        }
    }

    fn field_data(&mut self, tmpl: &'t XmlNode, scope: Option<&'t XmlNode>) -> Option<&'t XmlNode> {
        let name = tmpl.attr("name").unwrap_or("");
        let (mode, reference) = self.binding(tmpl);
        match mode {
            "none" => None,
            "global" => self.global(name).into_iter().next(),
            "dataRef" => reference
                .map(|reference| self.resolve_ref(reference, scope))
                .unwrap_or_default()
                .into_iter()
                .next(),
            _ => {
                let scope = scope?;
                let found = scope.children.iter().find(|child| {
                    child.local_name == name && !self.consumed.contains(&key(child))
                })?;
                self.consumed.insert(key(found));
                Some(found)
            }
        }
    }

    fn merge_field(&mut self, tmpl: &'t XmlNode, scope: Option<&'t XmlNode>) -> FormNode<'t> {
        let mut node = FormNode::new(NodeKind::Field, tmpl);
        node.items = self.bind_items(tmpl);
        let (value, rich, image) = template_value(self.tpl, tmpl);
        node.value = value;
        node.rich = rich;
        node.image = image;
        if let Some(data) = self.field_data(tmpl, scope) {
            node.data = Some(data);
            let is_image =
                node.image.is_some() || self.tpl.path(tmpl, &["ui", "imageEdit"]).is_some();
            if is_image {
                let text = data.text.trim();
                let href = data.attr("href").filter(|href| !href.is_empty());
                if !text.is_empty() || href.is_some() {
                    node.image = Some(ImageValue {
                        content_type: data.attr("contentType").unwrap_or("").to_string(),
                        data: text.to_string(),
                        href: href.map(str::to_string),
                    });
                }
            } else {
                node.value = Some(data_value(data));
                node.rich = None;
            }
        }
        node
    }

    fn merge_excl_group(&mut self, tmpl: &'t XmlNode, scope: Option<&'t XmlNode>) -> FormNode<'t> {
        let mut node = FormNode::new(NodeKind::ExclGroup, tmpl);
        let data = self.field_data(tmpl, scope);
        node.data = data;
        node.value = data.map(data_value);
        // Members take their on/off state from the group value; they keep
        // their template defaults when the group has no data.
        node.children = self.merge_children(tmpl, None);
        node
    }

    fn global(&self, name: &str) -> Vec<&'t XmlNode> {
        let mut out = Vec::new();
        let Some(data) = self.data else {
            return out;
        };
        if name.is_empty() {
            return out;
        }
        let mut stack = vec![data];
        while let Some(node) = stack.pop() {
            if node.local_name == name && !std::ptr::eq(node, data) {
                out.push(node);
                if out.len() > 10_000 {
                    break;
                }
            }
            stack.extend(node.children.iter().rev());
        }
        out
    }

    /// Resolves a bind `ref` such as `$.a.b[*]`, `$record.x`, `$data.y[0]`
    /// or `a.b` relative to `scope`.
    fn resolve_ref(&self, reference: &str, scope: Option<&'t XmlNode>) -> Vec<&'t XmlNode> {
        let reference = reference.trim();
        let (start, rest) = if let Some(rest) = reference.strip_prefix("$record") {
            (self.record_root(), rest)
        } else if let Some(rest) = reference.strip_prefix("$data") {
            (self.data, rest)
        } else if let Some(rest) = reference.strip_prefix("!") {
            (self.data, rest)
        } else if let Some(rest) = reference.strip_prefix('$') {
            (scope, rest)
        } else {
            (scope, reference)
        };
        let Some(start) = start else {
            return Vec::new();
        };
        let mut current = vec![start];
        for segment in rest.split('.').filter(|segment| !segment.is_empty()) {
            let (name, index) = match segment.find('[') {
                Some(open) => {
                    let inner = segment[open + 1..].trim_end_matches(']').trim();
                    let index = if inner == "*" {
                        None
                    } else {
                        Some(inner.parse::<usize>().unwrap_or(0))
                    };
                    (&segment[..open], index)
                }
                None => (segment, Some(0)),
            };
            let name = name.trim_start_matches('#');
            let mut next = Vec::new();
            for node in &current {
                let matches: Vec<&'t XmlNode> = node
                    .children
                    .iter()
                    .filter(|child| child.local_name == name)
                    .collect();
                match index {
                    Some(index) => next.extend(matches.get(index).copied()),
                    None => next.extend(matches),
                }
            }
            current = next;
            if current.is_empty() {
                break;
            }
        }
        current
    }

    /// `<bindItems ref labelRef valueRef>`: choice items from data nodes.
    fn bind_items(&self, tmpl: &'t XmlNode) -> Option<Vec<(String, String)>> {
        let bind = self.tpl.child(tmpl, "bindItems")?;
        let reference = self.tpl.attr(bind, "ref")?.trim();
        let (start, rest) = if let Some(rest) = reference.strip_prefix("xfa.datasets.data.") {
            (self.data, rest)
        } else if let Some(rest) = reference.strip_prefix("xfa.datasets.") {
            (self.datasets, rest)
        } else if let Some(rest) = reference.strip_prefix("$data.") {
            (self.data, rest)
        } else if let Some(rest) = reference.strip_prefix("$record.") {
            (self.record_root(), rest)
        } else {
            (self.data, reference.trim_start_matches("$."))
        };
        let nodes = self.resolve_ref(rest, start);
        let pick = |node: &XmlNode, reference: Option<&str>| -> String {
            match reference.map(str::trim) {
                None | Some("$") | Some("") => node.text.trim().to_string(),
                Some(name) => {
                    let name = name.trim_start_matches("$.").trim_start_matches('@');
                    node.attr(name)
                        .map(str::to_string)
                        .or_else(|| node.child(name).map(|child| child.text.trim().to_string()))
                        .unwrap_or_default()
                }
            }
        };
        let label = self.tpl.attr(bind, "labelRef");
        let value = self.tpl.attr(bind, "valueRef");
        let items: Vec<(String, String)> = nodes
            .iter()
            .take(10_000)
            .map(|node| (pick(node, label), pick(node, value.or(label))))
            .collect();
        (!items.is_empty()).then_some(items)
    }

    fn record_root(&self) -> Option<&'t XmlNode> {
        let data = self.data?;
        data.children.first()
    }
}

fn key(node: &XmlNode) -> usize {
    node as *const XmlNode as usize
}

/// A container's data binding: the `<bind match>` mode (`once`, `none`,
/// `global`, `dataRef`) and `ref`.
pub(crate) fn binding_mode<'t>(
    tpl: &Template<'t>,
    node: &'t XmlNode,
) -> (&'t str, Option<&'t str>) {
    let name = node.attr("name").unwrap_or("");
    let bind = tpl.child(node, "bind");
    let reference = bind.and_then(|bind| tpl.attr(bind, "ref"));
    let mode = bind
        .and_then(|bind| tpl.attr(bind, "match"))
        .unwrap_or(if reference.is_some() {
            "dataRef"
        } else if name.is_empty() {
            "none"
        } else {
            "once"
        });
    (mode, reference)
}

/// Value text of a data node: its character data (plain text of rich data).
pub(crate) fn data_value(node: &XmlNode) -> String {
    if node.children.is_empty() {
        node.text.trim_matches(['\r', '\n']).to_string()
    } else {
        rich_plain_text(node)
    }
}

/// The plain text of an XHTML fragment, with paragraph breaks.
pub(crate) fn rich_plain_text(node: &XmlNode) -> String {
    fn walk(node: &XmlNode, out: &mut String) {
        let block = matches!(node.local_name.as_str(), "p" | "div" | "li");
        if block && !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        if node.local_name == "br" {
            out.push('\n');
        }
        out.push_str(&node.head);
        for child in &node.children {
            walk(child, out);
            out.push_str(&child.tail);
        }
    }
    let mut out = String::new();
    walk(node, &mut out);
    out.trim().to_string()
}

/// The template default `<value>` of a field or draw.
pub(crate) fn template_value<'t>(
    tpl: &Template<'t>,
    node: &'t XmlNode,
) -> (Option<String>, Option<&'t XmlNode>, Option<ImageValue>) {
    let Some(value) = tpl.child(node, "value") else {
        return (None, None, None);
    };
    let Some(content) = value.children.first() else {
        return (None, None, None);
    };
    match content.local_name.as_str() {
        "exData" => {
            let content_type = content.attr("contentType").unwrap_or("");
            if content_type.contains("html") || content_type.contains("xml") {
                (Some(rich_plain_text(content)), Some(content), None)
            } else {
                (Some(content.text.clone()), None, None)
            }
        }
        "image" => {
            let data = content.text.trim().to_string();
            let href = content.attr("href").filter(|href| !href.is_empty());
            if data.is_empty() && href.is_none() {
                return (None, None, None);
            }
            (
                None,
                None,
                Some(ImageValue {
                    content_type: content.attr("contentType").unwrap_or("").to_string(),
                    data,
                    href: href.map(str::to_string),
                }),
            )
        }
        "line" | "rectangle" | "arc" => (None, None, None),
        _ => {
            let text = content.text.clone();
            (Some(text), None, None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xfa::xml::parse_xml;
    use crate::xfa::XfaLimits;

    fn parse(xml: &str) -> XmlNode {
        parse_xml(xml.as_bytes(), &XfaLimits::default())
            .unwrap()
            .root
    }

    #[test]
    fn merges_repeating_subforms_and_field_values() {
        let template = parse(
            r#"<template><subform name="form1">
                <subform name="row"><occur min="0" max="-1"/><field name="a"><value><text>dflt</text></value></field></subform>
                <field name="total"/>
                <exclGroup name="g"><field name="x"/></exclGroup>
            </subform></template>"#,
        );
        let data = parse(
            r#"<data><form1><row><a>one</a></row><row><a>two</a></row><row/><total>9</total><g>2</g></form1></data>"#,
        );
        let tpl = Template::new(&template);
        let model = build_form_model(&tpl, &template, Some(&data)).unwrap();
        let rows: Vec<_> = model
            .root
            .children
            .iter()
            .filter(|node| node.name() == "row")
            .collect();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].children[0].value.as_deref(), Some("one"));
        assert_eq!(rows[1].children[0].value.as_deref(), Some("two"));
        assert_eq!(rows[2].children[0].value.as_deref(), Some("dflt"));
        let total = model.root.children.iter().find(|n| n.name() == "total");
        assert_eq!(total.unwrap().value.as_deref(), Some("9"));
        let group = model.root.children.iter().find(|n| n.name() == "g");
        assert_eq!(group.unwrap().value.as_deref(), Some("2"));
        assert_eq!(model.page_areas.len(), 1);
    }

    #[test]
    fn page_area_medium_and_content_areas() {
        let template = parse(
            r#"<template><subform name="f"><pageSet><pageArea name="P" id="P1">
                <contentArea x="0.25in" y="0.5in" w="8in" h="10in"/>
                <medium stock="letter" short="8.5in" long="11in" orientation="landscape"/>
                <draw name="d"/></pageArea></pageSet></subform></template>"#,
        );
        let tpl = Template::new(&template);
        let model = build_form_model(&tpl, &template, None).unwrap();
        let area = &model.page_areas[0];
        assert_eq!((area.width, area.height), (792.0, 612.0));
        assert_eq!(area.content_areas[0], [18.0, 36.0, 576.0, 720.0]);
        assert_eq!(area.content.len(), 1);
    }

    #[test]
    fn prototype_properties_are_inherited() {
        let template = parse(
            r##"<template><subform name="f"><proto><font id="big" size="20pt" typeface="Arial"/></proto>
               <field name="a"><font use="#big"/></field></subform></template>"##,
        );
        let tpl = Template::new(&template);
        let root = template_root(&template).unwrap();
        let field = root.child("field").unwrap();
        let font = tpl.child(field, "font").unwrap();
        assert_eq!(tpl.attr(font, "typeface"), Some("Arial"));
        assert_eq!(tpl.measure(font, "size"), Some(20.0));
    }

    #[test]
    fn bind_items_read_choice_lists_from_datasets() {
        let template = parse(
            r#"<template><subform name="f"><field name="c"><ui><choiceList/></ui>
               <bindItems ref="xfa.datasets.LOV.C[*]" labelRef="$" valueRef="lic"/></field></subform></template>"#,
        );
        let datasets = parse(
            r#"<datasets><LOV><C lic="1">One</C><C lic="2">Two</C></LOV><data><f><c>2</c></f></data></datasets>"#,
        );
        let tpl = Template::new(&template);
        let model =
            build_form_model_with_datasets(&tpl, &template, data_root(&datasets), Some(&datasets))
                .unwrap();
        let field = &model.root.children[0];
        assert_eq!(field.value.as_deref(), Some("2"));
        assert_eq!(
            field.items.as_deref(),
            Some(
                &[
                    ("One".to_string(), "1".to_string()),
                    ("Two".to_string(), "2".to_string())
                ][..]
            )
        );
    }

    #[test]
    fn rich_text_keeps_inline_spacing() {
        let node = parse(r#"<body><p>Hello <b>big</b> <i>world</i></p><p>next</p></body>"#);
        assert_eq!(rich_plain_text(&node), "Hello big world\nnext");
    }
}
