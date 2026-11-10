//! XFA event scripts for rendering (PDFium `CXFA_FFDocView` event flow):
//! copies the merged form into the script DOM, runs `initialize`,
//! `calculate` (to a fixed point), `form:ready` (`ready` on `$form`) and
//! `docReady` before layout and `layout:ready` (`ready` on `$layout`)
//! after it, then writes values, presence, captions, choice items,
//! colours and instance changes back into the form model.
//!
//! FormCalc runs in [`super::formcalc`]; JavaScript in [`super::js`]
//! (boa, when the `xfa-js` feature is enabled on a non-wasm target).
//! Interactive events (click, change, enter, exit...), `validate` and
//! scripts marked `runAt="server"` never run.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use super::form::{
    assign_locales, fresh_node, instance_prototype, FormModel, FormNode, NodeKind, Template,
};
use super::layout::{caption_spec, field_ui, LayoutPage};
use super::locale::LocaleSet;
use super::som::{
    activity_kind, Activity, Class, Dom, Lang, LayoutInfo, Limits, Request, SNode, ScriptDef, Val,
    NO_TMPL,
};
use super::style::{parse_color, Presence};
use super::xml::XmlNode;

/// What the script pass did (for diagnostics and tests).
#[derive(Clone, Debug, Default)]
pub(crate) struct ScriptReport {
    pub executed: usize,
    pub failed: usize,
    /// JavaScript scripts skipped because no engine is available.
    pub javascript_skipped: usize,
    pub errors: Vec<String>,
    /// `app.alert` / `messageBox` output.
    pub log: Vec<String>,
}

const MAX_CALC_PASSES: usize = 12;
const PANICKED: &str = "XFA script engine failed; script changes discarded";
const MAX_EVENT_RUNS: usize = 100_000;

pub(crate) struct FormScripts<'t> {
    tmpls: Vec<&'t XmlNode>,
    tmpl_ids: HashMap<usize, usize>,
    /// Arena ids of the form nodes, in [`take_order`] order.
    form_ids: Vec<usize>,
    runner: Runner,
    /// Arena node id -> form node address after the last write-back.
    addresses: HashMap<usize, usize>,
}

fn key(node: &XmlNode) -> usize {
    node as *const XmlNode as usize
}

fn form_key(node: &FormNode<'_>) -> usize {
    node as *const FormNode<'_> as usize
}

impl<'t> FormScripts<'t> {
    /// Builds the script DOM when the template has any runnable script.
    pub(crate) fn prepare(
        tpl: &Template<'t>,
        template: &'t XmlNode,
        model: &FormModel<'t>,
        locales: LocaleSet,
    ) -> Option<Self> {
        if !has_scripts(template) {
            return None;
        }
        let mut tmpls = Vec::new();
        let mut tmpl_ids = HashMap::new();
        let mut stack = vec![(template, 0usize)];
        while let Some((node, depth)) = stack.pop() {
            tmpl_ids.insert(key(node), tmpls.len());
            tmpls.push(node);
            if depth < 256 {
                stack.extend(node.children.iter().rev().map(|child| (child, depth + 1)));
            }
        }
        let dom = Dom::new(Limits::default(), locales);
        let mut scripts = FormScripts {
            tmpls,
            tmpl_ids,
            form_ids: Vec::new(),
            runner: Runner::new(dom),
            addresses: HashMap::new(),
        };
        scripts.build(tpl, model);
        scripts.remember_addresses(model);
        Some(scripts)
    }

    fn tmpl_id(&self, node: &XmlNode) -> usize {
        self.tmpl_ids.get(&key(node)).copied().unwrap_or(NO_TMPL)
    }

    fn build(&mut self, tpl: &Template<'t>, model: &FormModel<'t>) {
        let root = self.add_form_node(tpl, &model.root, None, false, true);
        {
            let mut dom = self.runner.dom.borrow_mut();
            dom.root = root;
        }
        let root_tmpl = model.root.tmpl;
        let page_set = root_tmpl
            .children
            .iter()
            .find(|child| child.local_name == "pageSet");
        let page_set_id = {
            let mut node = SNode::new(
                Class::PageSet,
                page_set.and_then(|p| p.attr("name")).unwrap_or(""),
                page_set.map_or(NO_TMPL, |p| self.tmpl_id(p)),
            );
            node.presence = "visible".into();
            self.runner.dom.borrow_mut().add(node, Some(root))
        };
        for (index, area) in model.page_areas.iter().enumerate() {
            let mut node = SNode::new(Class::PageArea, area.name, self.tmpl_id(area.tmpl));
            node.aux = index;
            let area_id = self.runner.dom.borrow_mut().add(node, Some(page_set_id));
            for content in &area.content {
                self.add_form_node(tpl, content, Some(area_id), true, true);
            }
        }
        for child in &model.root.children {
            self.add_form_node(tpl, child, Some(root), true, true);
        }
        // Instance prototypes for repeatable subforms, when scripts may
        // add instances.
        if self.wants_prototypes() {
            let root_locale = model.root.locale;
            let repeatable: Vec<&'t XmlNode> = self
                .tmpls
                .iter()
                .copied()
                .filter(|node| node.local_name == "subform" && tpl.child(node, "occur").is_some())
                .take(512)
                .collect();
            for tmpl in repeatable {
                let mut proto = instance_prototype(tpl, tmpl);
                assign_locales(&mut proto, root_locale);
                let id = self.add_form_node(tpl, &proto, None, true, false);
                let tmpl_id = self.tmpl_id(tmpl);
                self.runner.dom.borrow_mut().protos.insert(tmpl_id, id);
            }
        }
    }

    fn wants_prototypes(&self) -> bool {
        let dom = self.runner.dom.borrow();
        dom.nodes.iter().any(|node| {
            node.scripts
                .iter()
                .any(|s| s.source.contains("Instance") || s.source.contains(".count"))
        })
    }

    fn add_form_node(
        &mut self,
        tpl: &Template<'t>,
        node: &FormNode<'t>,
        parent: Option<usize>,
        recurse: bool,
        record: bool,
    ) -> usize {
        let snode = self.snode_for(tpl, node);
        let id = self.runner.dom.borrow_mut().add(snode, parent);
        if record {
            self.form_ids.push(id);
        }
        self.add_variables(tpl, node.tmpl, id);
        if recurse {
            for child in &node.children {
                self.add_form_node(tpl, child, Some(id), true, record);
            }
        }
        id
    }

    fn add_variables(&mut self, tpl: &Template<'t>, tmpl: &'t XmlNode, parent: usize) {
        let Some(variables) = tpl.child(tmpl, "variables") else {
            return;
        };
        let mut dom = self.runner.dom.borrow_mut();
        for child in variables.children.iter().take(1024) {
            let Some(name) = child.attr("name") else {
                continue;
            };
            if child.local_name == "script" {
                let mut node = SNode::new(Class::ScriptObject, name, NO_TMPL);
                node.scripts.push(ScriptDef {
                    activity: Activity::Other,
                    activity_name: String::new(),
                    lang: script_lang(child),
                    source: child.text.clone(),
                });
                dom.add(node, Some(parent));
            } else {
                let mut node = SNode::new(Class::Variable, name, NO_TMPL);
                node.value = Some(child.text.trim().to_string());
                dom.add(node, Some(parent));
            }
        }
    }

    fn snode_for(&self, tpl: &Template<'t>, node: &FormNode<'t>) -> SNode {
        let tmpl = node.tmpl;
        let class = match node.kind {
            NodeKind::Subform => Class::Subform,
            NodeKind::Area => Class::Area,
            NodeKind::ExclGroup => Class::ExclGroup,
            NodeKind::Field => Class::Field,
            NodeKind::Draw => Class::Draw,
        };
        let mut s = SNode::new(class, tmpl.attr("name").unwrap_or(""), self.tmpl_id(tmpl));
        s.value = node.value.clone();
        s.value_kind = tpl
            .child(tmpl, "value")
            .and_then(|value| value.children.first())
            .map(|content| match content.local_name.as_str() {
                "exData" => "text".to_string(),
                other => other.to_string(),
            })
            .unwrap_or_else(|| "text".to_string());
        if node.kind == NodeKind::Field {
            let (_, widget) = field_ui(tpl, tmpl);
            s.ui = widget.map(|w| w.local_name.clone()).unwrap_or_default();
            if s.ui == "numericEdit" && s.value_kind == "text" {
                s.value_kind = "decimal".into();
            }
            if s.ui == "dateTimeEdit" && s.value_kind == "text" {
                s.value_kind = "date".into();
            }
            if s.ui == "checkButton" {
                let values = template_item_texts(tmpl);
                s.on_value = Some(values.first().cloned().unwrap_or_else(|| "1".into()));
                s.off_value = Some(values.get(1).cloned().unwrap_or_else(|| "0".into()));
            }
            s.items = node
                .items
                .clone()
                .unwrap_or_else(|| template_choice_items(tmpl));
        }
        s.presence = tpl.attr(tmpl, "presence").unwrap_or("visible").to_string();
        s.access = tpl.attr(tmpl, "access").unwrap_or("open").to_string();
        if let Some(caption) = caption_spec(tpl, tmpl) {
            s.has_caption = true;
            s.caption = caption.text;
            if let Some(element) = tpl.child(tmpl, "caption") {
                s.caption_placement = tpl.attr(element, "placement").map(str::to_string);
                s.caption_reserve = tpl.attr(element, "reserve").map(str::to_string);
                s.caption_presence = tpl.attr(element, "presence").map(str::to_string);
            }
        }
        s.picture = tpl
            .child(tmpl, "format")
            .and_then(|format| tpl.child(format, "picture"))
            .map(|picture| picture.text.trim().to_string())
            .filter(|picture| !picture.is_empty());
        s.locale = node.locale.map(str::to_string);
        for (slot, name) in s.geometry.iter_mut().zip(["x", "y", "w", "h"]) {
            *slot = tpl.attr(tmpl, name).map(str::to_string);
        }
        s.scripts = collect_scripts(tpl, tmpl);
        if node.kind == NodeKind::Subform {
            if let Some(occur) = tpl.child(tmpl, "occur") {
                s.occur = Some(parse_occur(tpl, occur));
            }
        }
        if matches!(node.kind, NodeKind::Subform | NodeKind::Area) {
            let mut managed = Vec::new();
            collect_managed(tpl, tmpl, &mut managed, 0);
            s.managed = managed
                .into_iter()
                .map(|(name, tmpl, min, max)| (name, self.tmpl_id(tmpl), min, max))
                .collect();
        }
        s
    }

    fn remember_addresses(&mut self, model: &FormModel<'t>) {
        self.addresses.clear();
        let order = visit_order(model);
        for (id, node) in self.form_ids.iter().zip(order) {
            self.addresses.insert(*id, form_key(node));
        }
    }

    /// Runs the pre-layout events and writes the results back.
    pub(crate) fn before_layout(&mut self, tpl: &Template<'t>, model: &mut FormModel<'t>) {
        let ran = self.runner.guarded(|runner| {
            runner.run_all(Activity::Initialize);
            runner.calculate_all(true);
            runner.run_all(Activity::FormReady);
            runner.run_all(Activity::DocReady);
            runner.calculate_all(false);
        });
        if ran {
            self.write_back(tpl, model);
        }
    }

    /// Whether any `layout:ready` script exists.
    pub(crate) fn has_layout_ready(&self) -> bool {
        let dom = self.runner.dom.borrow();
        dom.nodes.iter().any(|node| {
            node.scripts
                .iter()
                .any(|s| s.activity == Activity::LayoutReady)
        })
    }

    /// Layout facts for `xfa.layout`: page count and the page of every
    /// laid-out node.
    pub(crate) fn set_layout(&mut self, pages: &[LayoutPage<'_, 't>]) {
        let by_address: HashMap<usize, usize> = self
            .addresses
            .iter()
            .map(|(id, addr)| (*addr, *id))
            .collect();
        let mut info = LayoutInfo {
            page_count: pages.len(),
            ..LayoutInfo::default()
        };
        for (index, page) in pages.iter().enumerate() {
            for item in &page.items {
                if let Some(&id) = by_address.get(&form_key(item.node)) {
                    if !item.master {
                        info.node_pages.entry(id).or_insert(index);
                    }
                    info.node_rects
                        .entry(id)
                        .or_insert([item.x, item.y, item.w, item.h]);
                }
            }
        }
        self.runner.dom.borrow_mut().layout = Some(info);
    }

    /// Runs `layout:ready` on body (non-master-page) nodes; returns whether
    /// the form changed (and needs another layout).
    pub(crate) fn layout_ready(&mut self, tpl: &Template<'t>, model: &mut FormModel<'t>) -> bool {
        let masters = self.master_nodes();
        let before = self.runner.dom.borrow().mutations;
        let order = self.runner.post_order();
        let ran = self.runner.guarded(|runner| {
            for id in order {
                if masters.contains(&id) {
                    continue;
                }
                runner.run_node(id, Activity::LayoutReady, None);
            }
            runner.calculate_all(false);
        });
        let changed = ran && self.runner.dom.borrow().mutations != before;
        if changed {
            self.write_back(tpl, model);
        }
        changed
    }

    fn master_nodes(&self) -> HashSet<usize> {
        let dom = self.runner.dom.borrow();
        let mut out = HashSet::new();
        let mut stack: Vec<usize> = dom
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.class == Class::PageArea)
            .map(|(id, _)| id)
            .collect();
        while let Some(id) = stack.pop() {
            out.insert(id);
            stack.extend(dom.nodes[id].children.iter().copied());
        }
        out
    }

    /// Per-page values of master-page nodes whose `layout:ready` scripts
    /// depend on the page (`xfa.layout.page(this)`): (form node address,
    /// page index) -> value.
    pub(crate) fn master_page_values(
        &mut self,
        pages: &[LayoutPage<'_, 't>],
    ) -> HashMap<(usize, usize), String> {
        let mut out = HashMap::new();
        let masters = self.master_nodes();
        let scripted: Vec<usize> = {
            let dom = self.runner.dom.borrow();
            masters
                .iter()
                .copied()
                .filter(|&id| {
                    dom.nodes[id]
                        .scripts
                        .iter()
                        .any(|s| matches!(s.activity, Activity::LayoutReady))
                })
                .collect()
        };
        if scripted.is_empty() {
            return out;
        }
        let mut scripted = scripted;
        scripted.sort_unstable();
        for (index, page) in pages.iter().enumerate() {
            if !self.runner.dom.borrow().time_left() {
                break;
            }
            let on_page: HashSet<usize> = page
                .items
                .iter()
                .filter(|item| item.master)
                .map(|item| form_key(item.node))
                .collect();
            self.runner.dom.borrow_mut().current_page = Some(index);
            for &id in &scripted {
                let Some(&address) = self.addresses.get(&id) else {
                    continue;
                };
                if !on_page.contains(&address) {
                    continue;
                }
                if !self
                    .runner
                    .guarded(|runner| runner.run_node(id, Activity::LayoutReady, None))
                {
                    return out;
                }
                let dom = self.runner.dom.borrow();
                if let Some(value) = &dom.nodes[id].value {
                    out.insert((address, index), value.clone());
                }
            }
        }
        self.runner.dom.borrow_mut().current_page = None;
        out
    }

    pub(crate) fn report(&self) -> ScriptReport {
        let mut report = self.runner.report.clone();
        report.log = self.runner.dom.borrow().log.clone();
        report
    }

    /// Writes script changes into the form model (rebuilding it when
    /// instances were added or removed).
    fn write_back(&mut self, tpl: &Template<'t>, model: &mut FormModel<'t>) {
        let changed = {
            let dom = self.runner.dom.borrow();
            dom.structure_changed || dom.nodes.iter().any(|n| n.changes.any())
        };
        if !changed {
            return;
        }
        let taken = take_order(model);
        let mut payload: HashMap<usize, FormNode<'t>> = HashMap::new();
        for (id, node) in self.form_ids.iter().copied().zip(taken) {
            payload.insert(id, node);
        }
        let dom = Rc::clone(&self.runner.dom);
        let dom = dom.borrow();
        let root = dom.root;
        let mut form_ids = Vec::new();
        let mut root_node = payload
            .remove(&root)
            .unwrap_or_else(|| fresh_node(tpl, NodeKind::Subform, model.root.tmpl));
        apply_changes(&dom.nodes[root], &mut root_node);
        form_ids.push(root);
        let mut root_children = Vec::new();
        let mut areas: Vec<(usize, usize)> = Vec::new();
        for &child in &dom.nodes[root].children {
            match dom.nodes[child].class {
                Class::PageSet => {
                    for &area in &dom.nodes[child].children {
                        if dom.nodes[area].class == Class::PageArea {
                            areas.push((area, dom.nodes[area].aux));
                        }
                    }
                }
                Class::ScriptObject | Class::Variable => {}
                _ => root_children.push(child),
            }
        }
        let mut contents: Vec<(usize, Vec<FormNode<'t>>)> = Vec::new();
        for (area, index) in areas {
            let mut nodes = Vec::new();
            for &child in &dom.nodes[area].children {
                if let Some(node) = self.rebuild(tpl, &dom, child, &mut payload, &mut form_ids) {
                    nodes.push(node);
                }
            }
            contents.push((index, nodes));
        }
        let mut children = Vec::new();
        for child in root_children {
            if let Some(node) = self.rebuild(tpl, &dom, child, &mut payload, &mut form_ids) {
                children.push(node);
            }
        }
        root_node.children = children;
        model.root = root_node;
        for (index, nodes) in contents {
            if let Some(area) = model.page_areas.get_mut(index) {
                area.content = nodes;
            }
        }
        drop(dom);
        // The rebuild visited nodes in `take_order` order.
        self.form_ids = form_ids;
        assign_locales(&mut model.root, None);
        let root_locale = model.root.locale;
        for area in &mut model.page_areas {
            let locale = area.tmpl.attr("locale").or(root_locale);
            for node in &mut area.content {
                assign_locales(node, locale);
            }
        }
        {
            let mut dom = self.runner.dom.borrow_mut();
            for node in &mut dom.nodes {
                node.changes = Default::default();
            }
            dom.structure_changed = false;
        }
        self.remember_addresses(model);
    }

    fn rebuild(
        &self,
        tpl: &Template<'t>,
        dom: &Dom,
        id: usize,
        payload: &mut HashMap<usize, FormNode<'t>>,
        form_ids: &mut Vec<usize>,
    ) -> Option<FormNode<'t>> {
        let snode = &dom.nodes[id];
        let kind = match snode.class {
            Class::Subform => NodeKind::Subform,
            Class::Area => NodeKind::Area,
            Class::ExclGroup => NodeKind::ExclGroup,
            Class::Field => NodeKind::Field,
            Class::Draw => NodeKind::Draw,
            _ => return None,
        };
        let tmpl = *self.tmpls.get(snode.tmpl)?;
        let mut node = match payload.remove(&id) {
            Some(node) => node,
            None => {
                // A new instance: template defaults plus the script DOM's
                // current state.
                let mut node = fresh_node(tpl, kind, tmpl);
                if node.value != snode.value {
                    node.value = snode.value.clone();
                    node.rich = None;
                }
                node
            }
        };
        apply_changes(snode, &mut node);
        form_ids.push(id);
        let mut children = Vec::new();
        for &child in &snode.children {
            if let Some(child) = self.rebuild(tpl, dom, child, payload, form_ids) {
                children.push(child);
            }
        }
        node.children = children;
        Some(node)
    }
}

fn apply_changes(snode: &SNode, node: &mut FormNode<'_>) {
    let changes = &snode.changes;
    if changes.value && node.value != snode.value {
        node.value = snode.value.clone();
        node.rich = None;
    }
    if changes.presence {
        node.presence = Some(parse_presence(&snode.presence));
    }
    if changes.caption {
        node.caption = snode.caption.clone();
        node.caption_presence = snode.caption_presence.as_deref().map(parse_presence);
    }
    if changes.items {
        node.items = Some(snode.items.clone());
    }
    if changes.colors {
        node.fill_color = snode.fill_color.as_deref().and_then(parse_color);
        node.border_color = snode.border_color.as_deref().and_then(parse_color);
        node.font_color = snode.font_color.as_deref().and_then(parse_color);
    }
}

fn parse_presence(text: &str) -> Presence {
    match text {
        "invisible" => Presence::Invisible,
        "hidden" | "inactive" => Presence::Hidden,
        _ => Presence::Visible,
    }
}

/// Form nodes in build order: the root (alone), every page area's content
/// (depth first), then the root's children (depth first).
fn visit_order<'a, 't>(model: &'a FormModel<'t>) -> Vec<&'a FormNode<'t>> {
    fn walk<'a, 't>(node: &'a FormNode<'t>, out: &mut Vec<&'a FormNode<'t>>) {
        out.push(node);
        for child in &node.children {
            walk(child, out);
        }
    }
    let mut out = vec![&model.root];
    for area in &model.page_areas {
        for node in &area.content {
            walk(node, &mut out);
        }
    }
    for child in &model.root.children {
        walk(child, &mut out);
    }
    out
}

/// Takes every form node out of `model` (children detached) in
/// [`visit_order`] order.
fn take_order<'t>(model: &mut FormModel<'t>) -> Vec<FormNode<'t>> {
    fn flatten<'t>(mut node: FormNode<'t>, out: &mut Vec<FormNode<'t>>) {
        let children = std::mem::take(&mut node.children);
        out.push(node);
        for child in children {
            flatten(child, out);
        }
    }
    let tmpl = model.root.tmpl;
    let mut root = std::mem::replace(&mut model.root, FormNode::new(NodeKind::Subform, tmpl));
    let children = std::mem::take(&mut root.children);
    let mut out = vec![root];
    for area in &mut model.page_areas {
        for node in std::mem::take(&mut area.content) {
            flatten(node, &mut out);
        }
    }
    for child in children {
        flatten(child, &mut out);
    }
    out
}

fn has_scripts(template: &XmlNode) -> bool {
    template.descendants("script").next().is_some()
}

fn script_lang(script: &XmlNode) -> Lang {
    match script.attr("contentType").map(str::trim) {
        Some(kind) if kind.contains("javascript") || kind.contains("ecmascript") => {
            Lang::JavaScript
        }
        _ => Lang::FormCalc,
    }
}

fn runnable(script: &XmlNode) -> bool {
    !matches!(script.attr("runAt"), Some("server"))
}

fn collect_scripts<'t>(tpl: &Template<'t>, tmpl: &'t XmlNode) -> Vec<ScriptDef> {
    let mut out = Vec::new();
    for child in &tmpl.children {
        match child.local_name.as_str() {
            "event" => {
                let activity_name = child.attr("activity").unwrap_or("click").to_string();
                let activity = activity_kind(&activity_name, child.attr("ref"));
                if let Some(script) = child.child("script").filter(|s| runnable(s)) {
                    out.push(ScriptDef {
                        activity,
                        activity_name,
                        lang: script_lang(script),
                        source: script.text.clone(),
                    });
                }
            }
            "calculate" => {
                if tpl.attr(child, "override") == Some("disabled") {
                    continue;
                }
                if let Some(script) = child.child("script").filter(|s| runnable(s)) {
                    out.push(ScriptDef {
                        activity: Activity::Calculate,
                        activity_name: "calculate".into(),
                        lang: script_lang(script),
                        source: script.text.clone(),
                    });
                }
            }
            _ => {}
        }
    }
    // A prototype's calculate (`use="#proto"`) applies when the node has none.
    if !out.iter().any(|s| s.activity == Activity::Calculate) {
        if let Some(calculate) = tpl.child(tmpl, "calculate") {
            if tpl.attr(calculate, "override") != Some("disabled") {
                if let Some(script) = calculate.child("script").filter(|s| runnable(s)) {
                    out.push(ScriptDef {
                        activity: Activity::Calculate,
                        activity_name: "calculate".into(),
                        lang: script_lang(script),
                        source: script.text.clone(),
                    });
                }
            }
        }
    }
    out
}

fn parse_occur<'t>(tpl: &Template<'t>, occur: &'t XmlNode) -> (usize, Option<usize>) {
    let int = |name: &str| {
        tpl.attr(occur, name)
            .and_then(|value| value.trim().parse::<i64>().ok())
    };
    let min = int("min").unwrap_or(1).clamp(0, 10_000) as usize;
    let max = match int("max") {
        Some(max) if max < 0 => None,
        Some(max) => Some((max as usize).max(min)),
        None => Some(min.max(1)),
    };
    (min, max)
}

fn collect_managed<'t>(
    tpl: &Template<'t>,
    tmpl: &'t XmlNode,
    out: &mut Vec<(String, &'t XmlNode, usize, Option<usize>)>,
    depth: usize,
) {
    if depth > 8 {
        return;
    }
    for child in &tmpl.children {
        match child.local_name.as_str() {
            "subform" => {
                if let (Some(name), Some(occur)) = (child.attr("name"), tpl.child(child, "occur")) {
                    let (min, max) = parse_occur(tpl, occur);
                    out.push((name.to_string(), child, min, max));
                }
            }
            "subformSet" => collect_managed(tpl, child, out, depth + 1),
            _ => {}
        }
    }
}

fn template_item_texts(tmpl: &XmlNode) -> Vec<String> {
    tmpl.child("items")
        .map(|items| {
            items
                .children
                .iter()
                .map(|item| item.text.trim().to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn template_choice_items(tmpl: &XmlNode) -> Vec<(String, String)> {
    let lists: Vec<&XmlNode> = tmpl
        .children
        .iter()
        .filter(|child| child.local_name == "items")
        .collect();
    let texts = |items: &XmlNode| -> Vec<String> {
        items
            .children
            .iter()
            .map(|item| item.text.trim().to_string())
            .collect()
    };
    let (display, save) = match lists.as_slice() {
        [] => (Vec::new(), Vec::new()),
        [only] => (texts(only), texts(only)),
        [first, second, ..] => {
            if first.attr("save") == Some("1") {
                (texts(second), texts(first))
            } else {
                (texts(first), texts(second))
            }
        }
    };
    display
        .into_iter()
        .zip(save.into_iter().chain(std::iter::repeat(String::new())))
        .map(|(d, s)| {
            let s = if s.is_empty() { d.clone() } else { s };
            (d, s)
        })
        .collect()
}

/// Executes scripts against the DOM (FormCalc directly, JavaScript
/// through the optional engine).
struct Runner {
    dom: Rc<RefCell<Dom>>,
    #[cfg(all(feature = "xfa-js", not(target_arch = "wasm32")))]
    js: Option<super::js::JsRuntime>,
    #[cfg(all(feature = "xfa-js", not(target_arch = "wasm32")))]
    js_dead: bool,
    report: ScriptReport,
    runs: usize,
    /// Values each calculate script read when it last ran.
    calc_reads: HashMap<usize, HashSet<usize>>,
}

impl Runner {
    fn new(dom: Dom) -> Self {
        Self {
            dom: Rc::new(RefCell::new(dom)),
            #[cfg(all(feature = "xfa-js", not(target_arch = "wasm32")))]
            js: None,
            #[cfg(all(feature = "xfa-js", not(target_arch = "wasm32")))]
            js_dead: false,
            report: ScriptReport::default(),
            runs: 0,
            calc_reads: HashMap::new(),
        }
    }

    /// Runs `phase`, containing a panic from a script engine: the
    /// scripts' changes are then discarded and no further script runs.
    fn guarded(&mut self, phase: impl FnOnce(&mut Self)) -> bool {
        if self.dom.borrow().exhausted && self.report.errors.iter().any(|e| e == PANICKED) {
            return false;
        }
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| phase(self)));
        if outcome.is_ok() {
            return true;
        }
        self.report.errors.push(PANICKED.to_string());
        self.report.failed += 1;
        #[cfg(all(feature = "xfa-js", not(target_arch = "wasm32")))]
        {
            self.js = None;
            self.js_dead = true;
        }
        if let Ok(mut dom) = self.dom.try_borrow_mut() {
            dom.exhausted = true;
            dom.requests.clear();
        }
        false
    }

    /// Live (attached) nodes in post-order (children before parents, as
    /// PDFium's `ExecEventActivityByDeepFirst`).
    fn post_order(&self) -> Vec<usize> {
        let dom = self.dom.borrow();
        let mut out = Vec::new();
        let mut stack = vec![(dom.root, false)];
        while let Some((id, visited)) = stack.pop() {
            if visited {
                out.push(id);
                continue;
            }
            stack.push((id, true));
            for &child in dom.nodes[id].children.iter().rev() {
                stack.push((child, false));
            }
        }
        out
    }

    /// Live nodes in document order.
    fn pre_order(&self) -> Vec<usize> {
        let dom = self.dom.borrow();
        let mut out = Vec::new();
        let mut stack = vec![dom.root];
        while let Some(id) = stack.pop() {
            out.push(id);
            stack.extend(dom.nodes[id].children.iter().rev().copied());
        }
        out
    }

    fn run_all(&mut self, activity: Activity) {
        for id in self.post_order() {
            if !self.dom.borrow().time_left() {
                return;
            }
            self.run_node(id, activity, None);
        }
    }

    /// Runs calculate scripts (all of them when `full`, else those that
    /// never ran or read a value changed since), then re-runs dependents of
    /// every change until values settle (PDFium's calculate queue).
    fn calculate_all(&mut self, full: bool) {
        let calcs: Vec<usize> = {
            let dom = self.dom.borrow();
            self.pre_order()
                .into_iter()
                .filter(|&id| {
                    dom.nodes[id]
                        .scripts
                        .iter()
                        .any(|s| s.activity == Activity::Calculate)
                })
                .collect()
        };
        let changed: HashSet<usize> = self.dom.borrow_mut().changed.drain(..).collect();
        let mut queue: Vec<usize> = calcs
            .iter()
            .copied()
            .filter(|id| full || self.depends(*id, &changed))
            .collect();
        for _ in 0..MAX_CALC_PASSES {
            if queue.is_empty() {
                return;
            }
            for &id in &queue {
                if !self.dom.borrow().time_left() {
                    return;
                }
                *self.dom.borrow().reads.borrow_mut() = Some(HashSet::new());
                self.run_node(id, Activity::Calculate, None);
                let mut reads = self
                    .dom
                    .borrow()
                    .reads
                    .borrow_mut()
                    .take()
                    .unwrap_or_default();
                reads.remove(&id);
                self.calc_reads.insert(id, reads);
            }
            let changed: HashSet<usize> = self.dom.borrow_mut().changed.drain(..).collect();
            queue = calcs
                .iter()
                .copied()
                .filter(|id| self.depends(*id, &changed))
                .collect();
        }
    }

    /// Whether the calculate script of `id` must run for `changed` values.
    fn depends(&self, id: usize, changed: &HashSet<usize>) -> bool {
        match self.calc_reads.get(&id) {
            None => true,
            Some(reads) => reads.iter().any(|read| changed.contains(read)),
        }
    }

    fn run_node(&mut self, id: usize, activity: Activity, name: Option<&str>) {
        let scripts: Vec<ScriptDef> = {
            let dom = self.dom.borrow();
            let Some(node) = dom.nodes.get(id) else {
                return;
            };
            node.scripts
                .iter()
                .filter(|s| match name {
                    Some(name) if activity == Activity::Other => s.activity_name == name,
                    _ => s.activity == activity,
                })
                .cloned()
                .collect()
        };
        for script in scripts {
            self.runs += 1;
            if self.runs > MAX_EVENT_RUNS || !self.dom.borrow().time_left() {
                return;
            }
            let result = self.exec(id, &script);
            if let Some(value) = result {
                if script.activity == Activity::Calculate {
                    let mut dom = self.dom.borrow_mut();
                    let value = match value {
                        Val::Obj(obj) => dom.value_of(&obj).unwrap_or(Val::Null),
                        other => other,
                    };
                    if matches!(
                        dom.nodes[id].class,
                        Class::Field | Class::ExclGroup | Class::Draw
                    ) {
                        if let Err(error) = dom.set_value(id, &value) {
                            self.report.errors.push(error.0);
                        }
                    }
                }
            }
            self.drain_requests();
        }
    }

    fn drain_requests(&mut self) {
        let mut guard = 0;
        loop {
            let request = {
                let mut dom = self.dom.borrow_mut();
                if dom.requests.is_empty() {
                    return;
                }
                dom.requests.remove(0)
            };
            guard += 1;
            if guard > 10_000 {
                self.dom.borrow_mut().requests.clear();
                return;
            }
            match request {
                Request::Event(id, activity, name) => {
                    self.run_node(id, activity, Some(&name));
                }
                Request::Instance(id) => {
                    let subtree = {
                        let dom = self.dom.borrow();
                        let mut out = Vec::new();
                        let mut stack = vec![(id, false)];
                        while let Some((node, visited)) = stack.pop() {
                            if visited {
                                out.push(node);
                                continue;
                            }
                            stack.push((node, true));
                            for &child in dom.nodes[node].children.iter().rev() {
                                stack.push((child, false));
                            }
                        }
                        out
                    };
                    for node in subtree {
                        self.run_node(node, Activity::Initialize, None);
                    }
                }
                Request::RecalculateAll => {
                    // The runner recalculates after every phase anyway.
                }
            }
        }
    }

    /// Runs one script; `None` when it failed or produced no value.
    fn exec(&mut self, id: usize, script: &ScriptDef) -> Option<Val> {
        match script.lang {
            Lang::FormCalc => {
                let result = {
                    let mut dom = self.dom.borrow_mut();
                    super::formcalc::run(&mut dom, id, &script.source)
                };
                match result {
                    Ok(value) => {
                        self.report.executed += 1;
                        Some(value)
                    }
                    Err(error) => {
                        self.report.failed += 1;
                        if self.report.errors.len() < 64 {
                            self.report.errors.push(error.0);
                        }
                        None
                    }
                }
            }
            Lang::JavaScript => self.exec_js(id, script),
        }
    }

    #[cfg(all(feature = "xfa-js", not(target_arch = "wasm32")))]
    fn exec_js(&mut self, id: usize, script: &ScriptDef) -> Option<Val> {
        if self.js_dead {
            self.report.javascript_skipped += 1;
            return None;
        }
        if self.js.is_none() {
            match super::js::JsRuntime::new(Rc::clone(&self.dom)) {
                Ok(runtime) => self.js = Some(runtime),
                Err(error) => {
                    self.js_dead = true;
                    self.report.errors.push(error);
                    self.report.javascript_skipped += 1;
                    return None;
                }
            }
        }
        let runtime = self.js.as_mut()?;
        match runtime.run(id, &script.source) {
            Ok(value) => {
                self.report.executed += 1;
                value
            }
            Err(super::js::JsFailure::Script(error)) => {
                self.report.failed += 1;
                if self.report.errors.len() < 64 {
                    self.report.errors.push(error);
                }
                None
            }
            Err(super::js::JsFailure::Aborted(error)) => {
                // Budget exhausted mid-script: the engine state is no longer
                // trustworthy, so no further JavaScript runs.
                self.report.failed += 1;
                self.report.errors.push(error);
                self.js = None;
                self.js_dead = true;
                None
            }
        }
    }

    #[cfg(not(all(feature = "xfa-js", not(target_arch = "wasm32"))))]
    fn exec_js(&mut self, _id: usize, _script: &ScriptDef) -> Option<Val> {
        self.report.javascript_skipped += 1;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xfa::form::build_form_model;
    use crate::xfa::xml::parse_xml;
    use crate::xfa::XfaLimits;

    fn parse(xml: &str) -> XmlNode {
        parse_xml(xml.as_bytes(), &XfaLimits::default())
            .unwrap()
            .root
    }

    fn find<'a, 't>(node: &'a FormNode<'t>, name: &str) -> Option<&'a FormNode<'t>> {
        if node.tmpl.attr("name") == Some(name) {
            return Some(node);
        }
        node.children.iter().find_map(|child| find(child, name))
    }

    #[test]
    fn formcalc_events_set_values_presence_captions_and_instances() {
        let template = parse(
            r#"<template><subform name="form1" layout="tb">
              <pageSet><pageArea name="P"><contentArea w="500pt" h="700pt"/>
                <field name="pageNo"><event activity="ready" ref="$layout"><script>$ = xfa.layout.page($)</script></event></field>
              </pageArea></pageSet>
              <subform name="row" layout="tb"><occur min="0" max="-1"/>
                <field name="qty"><value><decimal/></value></field>
                <field name="price"><value><decimal/></value></field>
                <field name="amount"><value><decimal/></value><calculate><script>qty * price</script></calculate></field>
              </subform>
              <field name="total"><value><decimal/></value><calculate><script>Sum(row[*].amount)</script></calculate></field>
              <field name="note"><caption><value><text>Old</text></value></caption>
                <event activity="initialize"><script>
                  $.caption.value.text = "New"
                  if (total > 10) then note.presence = "hidden" endif
                </script></event></field>
              <field name="grow"><event activity="docReady" ref="$host"><script>_extra.setInstances(2)</script></event></field>
              <subform name="extra"><occur min="0" max="3" initial="0"/><field name="e"><event activity="initialize"><script>$ = "init"</script></event></field></subform>
            </subform></template>"#,
        );
        let data = parse(
            r#"<data><form1><row><qty>2</qty><price>3</price></row><row><qty>1</qty><price>5.5</price></row></form1></data>"#,
        );
        let tpl = Template::new(&template);
        let mut model = build_form_model(&tpl, &template, Some(&data)).unwrap();
        let mut scripts =
            FormScripts::prepare(&tpl, &template, &model, LocaleSet::default()).unwrap();
        scripts.before_layout(&tpl, &mut model);
        let report = scripts.report();
        assert_eq!(report.failed, 0, "{:?}", report.errors);
        let amounts: Vec<_> = model
            .root
            .children
            .iter()
            .filter(|n| n.tmpl.attr("name") == Some("row"))
            .map(|row| find(row, "amount").unwrap().value.clone())
            .collect();
        assert_eq!(
            amounts,
            vec![Some("6".to_string()), Some("5.5".to_string())]
        );
        assert_eq!(
            find(&model.root, "total").unwrap().value.as_deref(),
            Some("11.5")
        );
        let note = find(&model.root, "note").unwrap();
        assert_eq!(note.caption.as_deref(), Some("New"));
        // `initialize` ran before `calculate`, when total was still empty.
        assert_eq!(note.presence, None);
        let extras: Vec<_> = model
            .root
            .children
            .iter()
            .filter(|n| n.tmpl.attr("name") == Some("extra"))
            .collect();
        assert_eq!(extras.len(), 2);
        assert_eq!(find(extras[1], "e").unwrap().value.as_deref(), Some("init"));
        // Order: the new instances sit after `grow`, before nothing else.
        let names: Vec<_> = model
            .root
            .children
            .iter()
            .map(|n| n.tmpl.attr("name").unwrap_or(""))
            .collect();
        assert_eq!(
            names,
            ["row", "row", "total", "note", "grow", "extra", "extra"]
        );

        // layout:ready on master pages evaluates per page.
        let mut book = crate::xfa::text::FontBook::new(None, Vec::new());
        let pages = crate::xfa::layout::layout_form(&tpl, &model, &mut book);
        scripts.set_layout(&pages);
        let values = scripts.master_page_values(&pages);
        assert_eq!(values.len(), pages.len());
        assert!(values.values().all(|v| v == "1"));
    }
}
