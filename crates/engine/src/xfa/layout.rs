//! XFA layout: positioned and flowed containers (`position`, `tb`,
//! `lr-tb`, `rl-tb`, `table`/`row`), growable sizes, and pagination of the
//! form into the page set's content areas with `breakBefore`/`breakAfter`
//! (and legacy `<break>`) directives and overflow onto new pages — a
//! compact take on PDFium's `CXFA_ContentLayoutProcessor` and
//! `CXFA_ViewLayoutProcessor`.

use super::form::{FormModel, FormNode, NodeKind, PagePosition, Template};
use super::style::{presence, rotation, CaptionPlacement, FontSpec, Insets, ParaSpec, Presence};
use super::text::{layout_paragraphs, plain_paragraphs, rich_paragraphs, FontBook, Paragraph};
use super::xml::XmlNode;

/// Hard caps protecting against hostile templates.
const MAX_PAGES: usize = 2_000;
const MAX_PLACED: usize = 400_000;

/// A laid-out form node: its layout extent on the page (after rotation),
/// in points from the page's top-left corner.
#[derive(Clone, Debug)]
pub(crate) struct Placed<'f, 't> {
    pub node: &'f FormNode<'t>,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub rotate: i32,
    /// The last fragment of a container split across content areas.
    pub last: bool,
    /// Master-page (page area) content.
    pub master: bool,
}

#[derive(Debug)]
pub(crate) struct LayoutPage<'f, 't> {
    pub page_area: usize,
    pub width: f64,
    pub height: f64,
    pub items: Vec<Placed<'f, 't>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flow {
    Position,
    TopBottom,
    LeftRight,
    RightLeft,
    Table,
    Row,
}

fn flow_of<'t>(tpl: &Template<'t>, node: &'t XmlNode) -> Flow {
    match tpl.attr(node, "layout") {
        Some("tb") => Flow::TopBottom,
        Some("lr-tb") => Flow::LeftRight,
        Some("rl-tb") => Flow::RightLeft,
        Some("table") => Flow::Table,
        Some("row" | "rl-row") => Flow::Row,
        _ => Flow::Position,
    }
}

/// The UI widget of a field.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Ui {
    Text {
        multi_line: bool,
        comb: Option<usize>,
    },
    Numeric,
    DateTime,
    Password(char),
    Choice {
        list: bool,
    },
    Check {
        round: bool,
        size: f64,
        mark: Option<String>,
    },
    Button,
    Image,
    Signature,
    Barcode,
}

pub(crate) fn field_ui<'t>(tpl: &Template<'t>, node: &'t XmlNode) -> (Ui, Option<&'t XmlNode>) {
    let ui_child = tpl.child(node, "ui").and_then(|ui| {
        ui.children
            .iter()
            .find(|child| !matches!(child.local_name.as_str(), "extras" | "picture"))
    });
    let Some(widget) = ui_child else {
        let value_kind = tpl
            .child(node, "value")
            .and_then(|value| value.children.first())
            .map(|child| child.local_name.as_str());
        let ui = match value_kind {
            Some("image") => Ui::Image,
            Some("integer" | "decimal" | "float") => Ui::Numeric,
            Some("date" | "time" | "dateTime") => Ui::DateTime,
            _ => Ui::Text {
                multi_line: false,
                comb: None,
            },
        };
        return (ui, None);
    };
    let ui = match widget.local_name.as_str() {
        "textEdit" => Ui::Text {
            multi_line: tpl.attr(widget, "multiLine") == Some("1"),
            comb: tpl.child(widget, "comb").map(|comb| {
                tpl.attr(comb, "numberOfCells")
                    .and_then(|cells| cells.trim().parse::<usize>().ok())
                    .unwrap_or(0)
            }),
        },
        "numericEdit" => Ui::Numeric,
        "dateTimeEdit" => Ui::DateTime,
        "passwordEdit" => Ui::Password(
            tpl.attr(widget, "passwordChar")
                .and_then(|value| value.chars().next())
                .unwrap_or('*'),
        ),
        "choiceList" => Ui::Choice {
            list: matches!(tpl.attr(widget, "open"), Some("always" | "multiSelect")),
        },
        "checkButton" => Ui::Check {
            round: tpl.attr(widget, "shape") == Some("round"),
            size: tpl.measure(widget, "size").unwrap_or(10.0).max(0.0),
            mark: tpl.attr(widget, "mark").map(str::to_string),
        },
        "button" => Ui::Button,
        "imageEdit" => Ui::Image,
        "signature" => Ui::Signature,
        "barcode" => Ui::Barcode,
        _ => Ui::Text {
            multi_line: false,
            comb: None,
        },
    };
    (ui, Some(widget))
}

/// A field's or draw's `<font>`.
pub(crate) fn node_font<'t>(tpl: &Template<'t>, node: &'t XmlNode) -> FontSpec {
    FontSpec::from_node(tpl, tpl.child(node, "font"), &FontSpec::default())
}

pub(crate) fn node_para<'t>(tpl: &Template<'t>, node: &'t XmlNode) -> ParaSpec {
    ParaSpec::from_node(tpl, tpl.child(node, "para"))
}

pub(crate) fn node_margin<'t>(tpl: &Template<'t>, node: &'t XmlNode) -> Insets {
    Insets::from_margin(tpl, tpl.child(node, "margin"))
}

#[derive(Clone, Debug)]
pub(crate) struct CaptionSpec<'t> {
    pub placement: CaptionPlacement,
    pub reserve: Option<f64>,
    pub visible: bool,
    pub hidden: bool,
    pub margin: Insets,
    pub font: FontSpec,
    pub para: ParaSpec,
    pub text: Option<String>,
    pub rich: Option<&'t XmlNode>,
}

pub(crate) fn caption_spec<'t>(tpl: &Template<'t>, node: &'t XmlNode) -> Option<CaptionSpec<'t>> {
    let caption = tpl.child(node, "caption")?;
    let placement = match tpl.attr(caption, "placement") {
        Some("top") => CaptionPlacement::Top,
        Some("right") => CaptionPlacement::Right,
        Some("bottom") => CaptionPlacement::Bottom,
        Some("inline") => CaptionPlacement::Inline,
        _ => CaptionPlacement::Left,
    };
    let reserve = tpl
        .measure(caption, "reserve")
        .filter(|value| *value >= 0.0);
    let presence = presence(tpl, caption);
    let base_font = node_font(tpl, node);
    let font = FontSpec::from_node(tpl, tpl.child(caption, "font"), &base_font);
    let para = ParaSpec::from_node(tpl, tpl.child(caption, "para"));
    let value = tpl.child(caption, "value");
    let content = value.and_then(|value| value.children.first());
    let (text, rich) = match content {
        Some(content) if content.local_name == "exData" => {
            (Some(super::form::rich_plain_text(content)), Some(content))
        }
        Some(content) => (Some(content.text.clone()), None),
        None => (None, None),
    };
    Some(CaptionSpec {
        placement,
        reserve,
        visible: presence == Presence::Visible,
        hidden: presence == Presence::Hidden,
        margin: Insets::from_margin(tpl, tpl.child(caption, "margin")),
        font,
        para,
        text: text.filter(|text| !text.is_empty()),
        rich,
    })
}

impl<'t> CaptionSpec<'t> {
    pub(crate) fn paragraphs(&self) -> Vec<Paragraph> {
        match (self.rich, &self.text) {
            (Some(rich), _) => rich_paragraphs(rich, &self.font, &self.para),
            (None, Some(text)) => plain_paragraphs(text, &self.font, &self.para),
            (None, None) => Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct BreakTarget {
    page_area: Option<usize>,
    content_area: bool,
}

#[derive(Default)]
struct Block<'f, 't> {
    w: f64,
    h: f64,
    items: Vec<Placed<'f, 't>>,
}

impl<'f, 't> Block<'f, 't> {
    fn shift(&mut self, dx: f64, dy: f64) {
        for item in &mut self.items {
            item.x += dx;
            item.y += dy;
        }
    }
}

pub(crate) struct Layouter<'a, 'f, 't, 'r> {
    tpl: &'a Template<'t>,
    model: &'f FormModel<'t>,
    book: &'a mut FontBook<'r>,
    pages: Vec<LayoutPage<'f, 't>>,
    area_uses: Vec<usize>,
    page_area: usize,
    content: usize,
    cursor: f64,
    used: bool,
    pending_break: Option<BreakTarget>,
    placed: usize,
    depth: usize,
}

/// Lays out the whole form into pages.
pub(crate) fn layout_form<'f, 't>(
    tpl: &Template<'t>,
    model: &'f FormModel<'t>,
    book: &mut FontBook<'_>,
) -> Vec<LayoutPage<'f, 't>> {
    let mut layouter = Layouter {
        tpl,
        model,
        book,
        pages: Vec::new(),
        area_uses: vec![0; model.page_areas.len()],
        page_area: 0,
        content: 0,
        cursor: 0.0,
        used: false,
        pending_break: None,
        placed: 0,
        depth: 0,
    };
    layouter.start_page(None);
    let root = &model.root;
    match flow_of(tpl, root.tmpl) {
        Flow::TopBottom | Flow::Table => {
            let width = layouter.area()[2];
            layouter.flow_container(root, 0.0, width, true);
        }
        _ => layouter.flow_positioned(root),
    }
    layouter.pages
}

impl<'f, 't> Layouter<'_, 'f, 't, '_> {
    fn area(&self) -> [f64; 4] {
        let area = &self.model.page_areas[self.page_area];
        area.content_areas
            .get(self.content)
            .copied()
            .unwrap_or([0.0, 0.0, area.width, area.height])
    }

    fn content_count(&self) -> usize {
        self.model.page_areas[self.page_area]
            .content_areas
            .len()
            .max(1)
    }

    fn choose_page_area(&self, target: Option<usize>) -> usize {
        if let Some(target) = target {
            return target;
        }
        let areas = &self.model.page_areas;
        let page_number = self.pages.len();
        if self.model.paginated {
            let wanted = if page_number == 0 {
                [PagePosition::First, PagePosition::Only]
            } else {
                [PagePosition::Rest, PagePosition::Rest]
            };
            if let Some(index) = areas
                .iter()
                .position(|area| wanted.contains(&area.position))
            {
                return index;
            }
            if let Some(index) = areas
                .iter()
                .position(|area| area.position == PagePosition::Any)
            {
                return index;
            }
            return 0;
        }
        if page_number == 0 {
            return 0;
        }
        // orderedOccurrence: stay on the current page area until its
        // maximum occurrence is used up, then move to the next one.
        let current = self.page_area;
        let max = areas[current].max_occur.unwrap_or(usize::MAX);
        if self.area_uses[current] < max {
            return current;
        }
        if current + 1 < areas.len() {
            return current + 1;
        }
        current
    }

    fn start_page(&mut self, target: Option<usize>) {
        if self.pages.len() >= MAX_PAGES {
            return;
        }
        let index = self.choose_page_area(target);
        self.page_area = index;
        self.area_uses[index] += 1;
        self.content = 0;
        self.cursor = 0.0;
        self.used = false;
        let model = self.model;
        let area = &model.page_areas[index];
        let mut page = LayoutPage {
            page_area: index,
            width: area.width,
            height: area.height,
            items: Vec::new(),
        };
        // Master-page content, positioned on the page.
        for node in &area.content {
            if let Some(mut block) = self.layout_positioned_child(node, area.width) {
                for item in &mut block.items {
                    item.master = true;
                }
                page.items.extend(block.items);
            }
        }
        self.pages.push(page);
    }

    fn next_area(&mut self, target: Option<BreakTarget>) {
        let to_page = target.is_some_and(|t| !t.content_area);
        let page_area = target.and_then(|t| t.page_area);
        if !to_page && page_area.is_none() && self.content + 1 < self.content_count() {
            self.content += 1;
            self.cursor = 0.0;
            self.used = false;
            return;
        }
        if self.pages.len() >= MAX_PAGES {
            // Keep stacking on the last page rather than growing without bound.
            self.cursor = 0.0;
            self.used = false;
            return;
        }
        self.start_page(page_area);
    }

    /// Applies a break: to a new content area/page when the current one
    /// has content, or re-targets an empty page to the requested page area.
    fn apply_break(&mut self, target: BreakTarget) {
        if self.used {
            self.next_area(Some(target));
            return;
        }
        if let Some(page_area) = target.page_area {
            if page_area != self.page_area && self.content == 0 {
                if let Some(page) = self.pages.pop() {
                    self.area_uses[page.page_area] =
                        self.area_uses[page.page_area].saturating_sub(1);
                }
                self.start_page(Some(page_area));
            }
        }
    }

    fn take_pending_break(&mut self) {
        if let Some(target) = self.pending_break.take() {
            self.apply_break(target);
        }
    }

    fn current_page(&mut self) -> &mut LayoutPage<'f, 't> {
        self.pages
            .last_mut()
            .expect("layout always has a current page")
    }

    fn place_block(&mut self, mut block: Block<'f, 't>, x_off: f64) {
        let area = self.area();
        if block.h > area[3] - self.cursor + 0.01 && self.used {
            self.next_area(None);
        }
        let area = self.area();
        block.shift(area[0] + x_off, area[1] + self.cursor);
        self.placed += block.items.len();
        if self.placed > MAX_PLACED {
            return;
        }
        self.cursor += block.h;
        self.used = true;
        self.current_page().items.extend(block.items);
    }

    fn break_target(&self, node: &'t XmlNode, before: bool) -> Option<BreakTarget> {
        let tpl = self.tpl;
        let resolve = |target: Option<&str>| -> Option<usize> {
            let target = target?.trim();
            let target = target.trim_start_matches('#');
            if target.is_empty() {
                return None;
            }
            let name = target.rsplit('.').next().unwrap_or(target);
            self.model
                .page_areas
                .iter()
                .position(|area| area.id == Some(target) || area.name == name)
        };
        let element = if before { "breakBefore" } else { "breakAfter" };
        if let Some(directive) = tpl.child(node, element) {
            let kind = tpl.attr(directive, "targetType").unwrap_or("auto");
            let target = resolve(tpl.attr(directive, "target"));
            return match kind {
                "pageArea" | "pageEven" | "pageOdd" => Some(BreakTarget {
                    page_area: target,
                    content_area: false,
                }),
                "contentArea" => Some(BreakTarget {
                    page_area: None,
                    content_area: true,
                }),
                _ => None,
            };
        }
        let legacy = tpl.child(node, "break")?;
        let (attr, target_attr) = if before {
            ("before", "beforeTarget")
        } else {
            ("after", "afterTarget")
        };
        let target = resolve(tpl.attr(legacy, target_attr));
        match tpl.attr(legacy, attr) {
            Some("pageArea" | "pageEven" | "pageOdd") => Some(BreakTarget {
                page_area: target,
                content_area: false,
            }),
            Some("contentArea") => Some(BreakTarget {
                page_area: None,
                content_area: true,
            }),
            _ => None,
        }
    }

    fn splittable(&self, node: &'f FormNode<'t>) -> bool {
        if node.kind != NodeKind::Subform {
            return false;
        }
        if !matches!(flow_of(self.tpl, node.tmpl), Flow::TopBottom | Flow::Table) {
            return false;
        }
        if self.tpl.measure(node.tmpl, "h").is_some() {
            return false;
        }
        let intact = self
            .tpl
            .child(node.tmpl, "keep")
            .and_then(|keep| self.tpl.attr(keep, "intact"));
        !matches!(intact, Some("contentArea" | "pageArea"))
    }

    /// Flows a top-to-bottom container across content areas, splitting it
    /// between children when it does not fit.
    fn flow_container(&mut self, node: &'f FormNode<'t>, x_off: f64, avail_w: f64, root: bool) {
        self.depth += 1;
        if self.depth > 64 || self.placed > MAX_PLACED {
            self.depth -= 1;
            return;
        }
        let tpl = self.tpl;
        let margin = node_margin(tpl, node.tmpl);
        let width = tpl.measure(node.tmpl, "w").unwrap_or(avail_w).max(0.0);
        let inner_w = (width - margin.left - margin.right).max(0.0);
        let columns =
            (flow_of(tpl, node.tmpl) == Flow::Table).then(|| self.column_widths(node, inner_w));
        let visible = presence(tpl, node.tmpl) != Presence::Hidden;
        if !visible {
            self.depth -= 1;
            return;
        }
        if !root {
            if let Some(target) = self.break_target(node.tmpl, true) {
                self.pending_break = Some(target);
            }
        }
        self.take_pending_break();
        let mut fragment = self.open_fragment(node, x_off, width);
        self.cursor += margin.top;
        for child in &node.children {
            if presence(tpl, child.tmpl) == Presence::Hidden {
                continue;
            }
            if self.placed > MAX_PLACED {
                break;
            }
            if let Some(target) = self.break_target(child.tmpl, true) {
                self.pending_break = Some(target);
            }
            if self.pending_break.is_some() {
                let before = self.pages.len() * 64 + self.content;
                self.take_pending_break();
                if self.pages.len() * 64 + self.content != before {
                    fragment = self.reopen_fragment(fragment, node, x_off, width);
                }
            }
            if self.splittable(child) {
                let before = self.pages.len() * 64 + self.content;
                self.flow_container(child, x_off + margin.left, inner_w, false);
                if self.pages.len() * 64 + self.content != before {
                    fragment = self.reopen_fragment_after_child(fragment, node, x_off, width);
                }
            } else {
                let block = match (&columns, flow_of(tpl, child.tmpl)) {
                    (Some(columns), Flow::Row) if child.kind == NodeKind::Subform => {
                        self.layout_row(child, columns)
                    }
                    _ => self.layout_node(child, inner_w),
                };
                if let Some(block) = block {
                    let area = self.area();
                    if block.h > area[3] - self.cursor + 0.01 && self.used {
                        self.close_fragment(&fragment, false);
                        self.next_area(None);
                        fragment = self.open_fragment(node, x_off, width);
                    }
                    self.place_block(block, x_off + margin.left);
                }
            }
            if let Some(target) = self.break_target(child.tmpl, false) {
                self.pending_break = Some(target);
            }
        }
        self.cursor += margin.bottom;
        self.close_fragment(&fragment, true);
        if !root {
            if let Some(target) = self.break_target(node.tmpl, false) {
                self.pending_break = Some(target);
            }
        }
        self.depth -= 1;
    }

    fn open_fragment(
        &mut self,
        node: &'f FormNode<'t>,
        x_off: f64,
        width: f64,
    ) -> (usize, usize, f64) {
        let area = self.area();
        let y = area[1] + self.cursor;
        let page = self.pages.len() - 1;
        let items = &mut self.current_page().items;
        items.push(Placed {
            node,
            x: area[0] + x_off,
            y,
            w: width,
            h: 0.0,
            rotate: 0,
            last: false,
            master: false,
        });
        (page, items.len() - 1, y)
    }

    fn close_fragment(&mut self, fragment: &(usize, usize, f64), last: bool) {
        let (page, index, y) = *fragment;
        let bottom = self.area()[1] + self.cursor;
        let current_page = self.pages.len() - 1;
        if let Some(item) = self
            .pages
            .get_mut(page)
            .and_then(|page| page.items.get_mut(index))
        {
            item.last = last;
            item.h = if page == current_page {
                (bottom - y).max(0.0)
            } else {
                item.h.max(0.0)
            };
        }
    }

    /// The content area changed while placing children (a break was taken):
    /// end the old fragment at the bottom of its area and open a new one.
    fn reopen_fragment(
        &mut self,
        fragment: (usize, usize, f64),
        node: &'f FormNode<'t>,
        x_off: f64,
        width: f64,
    ) -> (usize, usize, f64) {
        self.finish_old_fragment(&fragment);
        self.open_fragment(node, x_off, width)
    }

    fn reopen_fragment_after_child(
        &mut self,
        fragment: (usize, usize, f64),
        node: &'f FormNode<'t>,
        x_off: f64,
        width: f64,
    ) -> (usize, usize, f64) {
        self.finish_old_fragment(&fragment);
        // The new fragment starts at the top of the current area, behind the
        // child content already placed there.
        let area = self.area();
        let page = self.pages.len() - 1;
        let items = &mut self.current_page().items;
        let insert_at = items
            .iter()
            .position(|item| !item.master && item.y >= area[1] - 0.01)
            .unwrap_or(items.len());
        items.insert(
            insert_at,
            Placed {
                node,
                x: area[0] + x_off,
                y: area[1],
                w: width,
                h: 0.0,
                rotate: 0,
                last: false,
                master: false,
            },
        );
        (page, insert_at, area[1])
    }

    fn finish_old_fragment(&mut self, fragment: &(usize, usize, f64)) {
        let (page, index, y) = *fragment;
        let Some(page_ref) = self.pages.get(page) else {
            return;
        };
        let area_bottom = {
            let model_area = &self.model.page_areas[page_ref.page_area];
            // The fragment's content area: the one containing its top.
            model_area
                .content_areas
                .iter()
                .find(|area| y >= area[1] - 0.01 && y <= area[1] + area[3] + 0.01)
                .map(|area| area[1] + area[3])
                .unwrap_or(page_ref.height)
        };
        // Shrink to the content actually placed in it.
        let content_bottom = page_ref
            .items
            .iter()
            .skip(index + 1)
            .filter(|item| !item.master && item.y >= y - 0.01 && item.y <= area_bottom)
            .map(|item| item.y + item.h)
            .fold(y, f64::max);
        if let Some(item) = self
            .pages
            .get_mut(page)
            .and_then(|page| page.items.get_mut(index))
        {
            item.h = (content_bottom.min(area_bottom) - y).max(0.0);
            item.last = false;
        }
    }

    /// A positioned root: children keep their coordinates in the current
    /// content area; break directives start new areas/pages.
    fn flow_positioned(&mut self, root: &'f FormNode<'t>) {
        let tpl = self.tpl;
        for child in &root.children {
            if presence(tpl, child.tmpl) == Presence::Hidden {
                continue;
            }
            if let Some(target) = self.break_target(child.tmpl, true) {
                self.pending_break = Some(target);
            }
            self.take_pending_break();
            let area = self.area();
            if let Some(mut block) = self.layout_positioned_child(child, area[2]) {
                block.shift(area[0], area[1]);
                self.placed += block.items.len();
                self.used = true;
                self.cursor = self.cursor.max(block.h);
                self.current_page().items.extend(block.items);
            }
            if let Some(target) = self.break_target(child.tmpl, false) {
                self.pending_break = Some(target);
            }
            if self.placed > MAX_PLACED {
                break;
            }
        }
    }

    /// Lays out `node` at its own x/y (anchor-adjusted); the block's origin
    /// is the parent's content origin and `h`/`w` its far edges.
    fn layout_positioned_child(
        &mut self,
        node: &'f FormNode<'t>,
        avail_w: f64,
    ) -> Option<Block<'f, 't>> {
        let tpl = self.tpl;
        let mut block = self.layout_node(node, avail_w)?;
        let x = tpl.measure(node.tmpl, "x").unwrap_or(0.0);
        let y = tpl.measure(node.tmpl, "y").unwrap_or(0.0);
        let (dx, dy) = anchor_offset(
            tpl.attr(node.tmpl, "anchorType"),
            rotation(tpl, node.tmpl),
            block.w,
            block.h,
        );
        block.shift(x + dx, y + dy);
        let right = x + dx + block.w;
        let bottom = y + dy + block.h;
        block.w = right;
        block.h = bottom;
        Some(block)
    }

    fn column_widths(&mut self, node: &'f FormNode<'t>, inner_w: f64) -> Vec<f64> {
        let specified: Vec<Option<f64>> = self
            .tpl
            .attr(node.tmpl, "columnWidths")
            .unwrap_or("")
            .split_whitespace()
            .map(|value| {
                super::parse_measurement(value)
                    .ok()
                    .filter(|width| *width >= 0.0)
            })
            .collect();
        let fixed: f64 = specified.iter().flatten().sum();
        let auto = specified.iter().filter(|width| width.is_none()).count();
        let auto_width = if auto > 0 {
            ((inner_w - fixed) / auto as f64).max(0.0)
        } else {
            0.0
        };
        specified
            .into_iter()
            .map(|width| width.unwrap_or(auto_width))
            .collect()
    }

    /// A table row: cells left to right in the table's columns, all as tall
    /// as the tallest cell.
    fn layout_row(&mut self, node: &'f FormNode<'t>, columns: &[f64]) -> Option<Block<'f, 't>> {
        let tpl = self.tpl;
        if presence(tpl, node.tmpl) == Presence::Hidden {
            return None;
        }
        let margin = node_margin(tpl, node.tmpl);
        let mut cells: Vec<Block<'f, 't>> = Vec::new();
        let mut x = margin.left;
        let mut column = 0usize;
        let mut height = 0.0f64;
        for child in &node.children {
            if presence(tpl, child.tmpl) == Presence::Hidden {
                continue;
            }
            let span = tpl
                .attr(child.tmpl, "colSpan")
                .and_then(|span| span.trim().parse::<i64>().ok())
                .unwrap_or(1);
            let span = if span < 0 {
                columns.len().saturating_sub(column).max(1)
            } else {
                (span as usize).max(1)
            };
            let width: f64 = columns.iter().skip(column).take(span).sum();
            column += span;
            let Some(mut cell) = self.layout_node_sized(child, width, Some(width)) else {
                continue;
            };
            cell.shift(x, margin.top);
            x += width;
            height = height.max(cell.h);
            cells.push(cell);
        }
        let row_h = tpl
            .measure(node.tmpl, "h")
            .unwrap_or(height + margin.top + margin.bottom);
        let row_w = x + margin.right;
        let mut block = Block {
            w: row_w,
            h: row_h,
            items: vec![Placed {
                node,
                x: 0.0,
                y: 0.0,
                w: row_w,
                h: row_h,
                rotate: 0,
                last: true,
                master: false,
            }],
        };
        let cell_h = (row_h - margin.top - margin.bottom).max(height);
        for mut cell in cells {
            // Stretch the cell's own box to the row height.
            if let Some(first) = cell.items.first_mut() {
                first.h = cell_h;
            }
            block.items.append(&mut cell.items);
        }
        Some(block)
    }

    fn layout_node(&mut self, node: &'f FormNode<'t>, avail_w: f64) -> Option<Block<'f, 't>> {
        self.layout_node_sized(node, avail_w, None)
    }

    /// Lays out a node at the origin; `forced_w` overrides its width.
    fn layout_node_sized(
        &mut self,
        node: &'f FormNode<'t>,
        avail_w: f64,
        forced_w: Option<f64>,
    ) -> Option<Block<'f, 't>> {
        let tpl = self.tpl;
        if presence(tpl, node.tmpl) == Presence::Hidden {
            return None;
        }
        self.depth += 1;
        if self.depth > 96 {
            self.depth -= 1;
            return None;
        }
        let block = match node.kind {
            NodeKind::Field | NodeKind::Draw => {
                let rotate = rotation(tpl, node.tmpl);
                let (w, h) = self.leaf_size(node, avail_w, forced_w);
                let (ew, eh) = if rotate % 180 == 90 { (h, w) } else { (w, h) };
                Block {
                    w: ew,
                    h: eh,
                    items: vec![Placed {
                        node,
                        x: 0.0,
                        y: 0.0,
                        w: ew,
                        h: eh,
                        rotate,
                        last: true,
                        master: false,
                    }],
                }
            }
            NodeKind::Subform | NodeKind::Area | NodeKind::ExclGroup => {
                self.layout_container(node, avail_w, forced_w)
            }
        };
        self.depth -= 1;
        Some(block)
    }

    fn layout_container(
        &mut self,
        node: &'f FormNode<'t>,
        avail_w: f64,
        forced_w: Option<f64>,
    ) -> Block<'f, 't> {
        let tpl = self.tpl;
        let margin = node_margin(tpl, node.tmpl);
        let explicit_w = forced_w.or_else(|| tpl.measure(node.tmpl, "w"));
        let explicit_h = tpl.measure(node.tmpl, "h");
        let flow = if node.kind == NodeKind::Area {
            Flow::Position
        } else {
            flow_of(tpl, node.tmpl)
        };
        let width_for_children = explicit_w.unwrap_or(avail_w);
        let inner_w = (width_for_children - margin.left - margin.right).max(0.0);
        let mut items = Vec::new();
        let mut extent_w = 0.0f64;
        let mut extent_h = 0.0f64;
        match flow {
            Flow::Position => {
                for child in &node.children {
                    if let Some(mut block) = self.layout_positioned_child(child, inner_w) {
                        block.shift(margin.left, margin.top);
                        extent_w = extent_w.max(block.w);
                        extent_h = extent_h.max(block.h);
                        items.append(&mut block.items);
                    }
                }
            }
            Flow::TopBottom | Flow::Table => {
                let columns = (flow == Flow::Table).then(|| self.column_widths(node, inner_w));
                let mut y = 0.0;
                for child in &node.children {
                    let block = match (&columns, flow_of(tpl, child.tmpl)) {
                        (Some(columns), Flow::Row) if child.kind == NodeKind::Subform => {
                            self.layout_row(child, columns)
                        }
                        _ => self.layout_node(child, inner_w),
                    };
                    if let Some(mut block) = block {
                        block.shift(margin.left, margin.top + y);
                        y += block.h;
                        extent_w = extent_w.max(block.w);
                        items.append(&mut block.items);
                    }
                }
                extent_h = y;
            }
            Flow::LeftRight | Flow::RightLeft | Flow::Row => {
                let wrap = flow != Flow::Row;
                let mut x = 0.0f64;
                let mut y = 0.0f64;
                let mut row_h = 0.0f64;
                let mut row: Vec<Block<'f, 't>> = Vec::new();
                let mut rows: Vec<(Vec<Block<'f, 't>>, f64, f64)> = Vec::new();
                for child in &node.children {
                    let Some(block) = self.layout_node(child, inner_w) else {
                        continue;
                    };
                    if wrap && !row.is_empty() && x + block.w > inner_w + 0.01 {
                        rows.push((std::mem::take(&mut row), y, x));
                        y += row_h;
                        x = 0.0;
                        row_h = 0.0;
                    }
                    let mut block = block;
                    block.shift(x, 0.0);
                    x += block.w;
                    row_h = row_h.max(block.h);
                    extent_w = extent_w.max(x);
                    row.push(block);
                }
                if !row.is_empty() {
                    rows.push((row, y, x));
                    y += row_h;
                }
                extent_h = y;
                let container_inner = explicit_w
                    .map(|w| (w - margin.left - margin.right).max(0.0))
                    .unwrap_or(extent_w);
                for (row, row_y, row_w) in rows {
                    for mut block in row {
                        let dx = if flow == Flow::RightLeft {
                            // Mirror: the first child sits at the right edge.
                            let left = block.items.first().map_or(0.0, |item| item.x);
                            container_inner - row_w + (row_w - left - block.w) - left
                        } else {
                            0.0
                        };
                        block.shift(margin.left + dx, margin.top + row_y);
                        items.append(&mut block.items);
                    }
                }
            }
        }
        let min_w = tpl.measure(node.tmpl, "minW").unwrap_or(0.0);
        let max_w = tpl.measure(node.tmpl, "maxW").filter(|v| *v > 0.0);
        let min_h = tpl.measure(node.tmpl, "minH").unwrap_or(0.0);
        let max_h = tpl.measure(node.tmpl, "maxH").filter(|v| *v > 0.0);
        let w = explicit_w.unwrap_or_else(|| {
            let content = match flow {
                Flow::Position => extent_w + margin.left + margin.right,
                _ => extent_w + margin.left + margin.right,
            };
            clamp(content, min_w, max_w)
        });
        let h = explicit_h.unwrap_or_else(|| {
            let content = match flow {
                Flow::Position => extent_h + margin.top + margin.bottom,
                _ => extent_h + margin.top + margin.bottom,
            };
            clamp(content, min_h, max_h)
        });
        let mut all = Vec::with_capacity(items.len() + 1);
        all.push(Placed {
            node,
            x: 0.0,
            y: 0.0,
            w,
            h,
            rotate: 0,
            last: true,
            master: false,
        });
        all.append(&mut items);
        Block { w, h, items: all }
    }

    /// Size of a field or draw: explicit `w`/`h`, else measured content
    /// clamped to `minW`..`maxW` / `minH`..`maxH`.
    fn leaf_size(
        &mut self,
        node: &'f FormNode<'t>,
        avail_w: f64,
        forced_w: Option<f64>,
    ) -> (f64, f64) {
        let tpl = self.tpl;
        let explicit_w = forced_w.or_else(|| tpl.measure(node.tmpl, "w"));
        let explicit_h = tpl.measure(node.tmpl, "h");
        if let (Some(w), Some(h)) = (explicit_w, explicit_h) {
            return (w.max(0.0), h.max(0.0));
        }
        let min_w = tpl.measure(node.tmpl, "minW").unwrap_or(0.0);
        let max_w = tpl.measure(node.tmpl, "maxW").filter(|v| *v > 0.0);
        let min_h = tpl.measure(node.tmpl, "minH").unwrap_or(0.0);
        let max_h = tpl.measure(node.tmpl, "maxH").filter(|v| *v > 0.0);
        let margin = node_margin(tpl, node.tmpl);
        let font = node_font(tpl, node.tmpl);
        let para = node_para(tpl, node.tmpl);
        let caption = if node.kind == NodeKind::Field {
            caption_spec(tpl, node.tmpl).filter(|caption| !caption.hidden)
        } else {
            None
        };
        let (ui, _) = if node.kind == NodeKind::Field {
            field_ui(tpl, node.tmpl)
        } else {
            (
                Ui::Text {
                    multi_line: true,
                    comb: None,
                },
                None,
            )
        };
        let draw_shape = node.kind == NodeKind::Draw
            && tpl
                .child(node.tmpl, "value")
                .and_then(|value| value.children.first())
                .is_some_and(|content| {
                    matches!(
                        content.local_name.as_str(),
                        "line" | "rectangle" | "arc" | "image"
                    )
                });
        let paragraphs = match (node.rich, &node.value) {
            _ if draw_shape => Vec::new(),
            (Some(rich), _) => rich_paragraphs(rich, &font, &para),
            (None, Some(value)) if !value.is_empty() => plain_paragraphs(value, &font, &para),
            _ => Vec::new(),
        };
        let caption_paragraphs = caption
            .as_ref()
            .map(|caption| caption.paragraphs())
            .unwrap_or_default();
        let caption_size = if caption_paragraphs.is_empty() {
            (0.0, 0.0)
        } else {
            let block = layout_paragraphs(self.book, &caption_paragraphs, None);
            (block.width, block.height)
        };
        let (caption_w, caption_h) = match &caption {
            Some(caption) => {
                let reserve = caption.reserve.filter(|r| *r > 0.0);
                match caption.placement {
                    CaptionPlacement::Left | CaptionPlacement::Right => (
                        reserve
                            .unwrap_or(caption_size.0 + caption.margin.left + caption.margin.right),
                        caption_size.1,
                    ),
                    CaptionPlacement::Top | CaptionPlacement::Bottom => (
                        caption_size.0,
                        reserve
                            .unwrap_or(caption_size.1 + caption.margin.top + caption.margin.bottom),
                    ),
                    CaptionPlacement::Inline => (caption_size.0, 0.0),
                }
            }
            None => (0.0, 0.0),
        };
        let horizontal_caption = caption.as_ref().is_some_and(|caption| {
            matches!(
                caption.placement,
                CaptionPlacement::Left | CaptionPlacement::Right
            )
        });
        let w = match explicit_w {
            Some(w) => w,
            None => {
                let natural = layout_paragraphs(self.book, &paragraphs, None).width;
                let content = match ui {
                    Ui::Check { size, .. } => size,
                    _ => natural,
                };
                let content = if horizontal_caption {
                    content + caption_w
                } else {
                    content.max(caption_w)
                };
                let measured = content + margin.left + margin.right;
                let measured = if node.kind == NodeKind::Field {
                    measured + 4.0
                } else {
                    measured
                };
                clamp(measured.min(avail_w.max(min_w)), min_w, max_w)
            }
        };
        let h = match explicit_h {
            Some(h) => h,
            None => {
                let text_w = (w
                    - margin.left
                    - margin.right
                    - if horizontal_caption { caption_w } else { 0.0 })
                .max(1.0);
                let wrap = match ui {
                    Ui::Text { multi_line, .. } => multi_line || node.kind == NodeKind::Draw,
                    _ => false,
                };
                let content = match ui {
                    Ui::Check { size, .. } => size,
                    _ if paragraphs.is_empty() => font.size * 1.2,
                    _ => layout_paragraphs(self.book, &paragraphs, wrap.then_some(text_w)).height,
                };
                let content = if node.kind == NodeKind::Field {
                    content.max(font.size) + 4.0
                } else {
                    content
                };
                let content = if horizontal_caption {
                    content.max(caption_h)
                } else {
                    content + caption_h
                };
                clamp(content + margin.top + margin.bottom, min_h, max_h)
            }
        };
        (w.max(0.0), h.max(0.0))
    }
}

fn clamp(value: f64, min: f64, max: Option<f64>) -> f64 {
    let value = value.max(min);
    match max {
        Some(max) if max >= min => value.min(max),
        _ => value,
    }
}

/// PDFium `CalculatePositionedContainerPos`: the offset from a node's x/y
/// to the top-left of its layout extent (`w` x `h`, already rotated). The
/// anchor turns with the node (`kNextPosTable`).
fn anchor_offset(anchor: Option<&str>, rotate: i32, w: f64, h: f64) -> (f64, f64) {
    const NEXT_POS: [[usize; 9]; 4] = [
        [0, 1, 2, 3, 4, 5, 6, 7, 8],
        [6, 3, 0, 7, 4, 1, 8, 5, 2],
        [8, 7, 6, 5, 4, 3, 2, 1, 0],
        [2, 5, 8, 1, 4, 7, 0, 3, 6],
    ];
    let anchor = match anchor.unwrap_or("topLeft") {
        "topCenter" => 1,
        "topRight" => 2,
        "middleLeft" => 3,
        "middleCenter" => 4,
        "middleRight" => 5,
        "bottomLeft" => 6,
        "bottomCenter" => 7,
        "bottomRight" => 8,
        _ => 0,
    };
    let absolute = NEXT_POS[(rotate / 90).rem_euclid(4) as usize][anchor];
    let dy = match absolute / 3 {
        1 => -h / 2.0,
        2 => -h,
        _ => 0.0,
    };
    let dx = match absolute % 3 {
        1 => -w / 2.0,
        2 => -w,
        _ => 0.0,
    };
    (dx, dy)
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

    const PAGE_SET: &str = r#"<pageSet><pageArea name="P1" id="P1"><contentArea x="36pt" y="36pt" w="540pt" h="200pt"/><medium short="612pt" long="272pt"/></pageArea>
        <pageArea name="P2" id="P2"><contentArea x="0pt" y="0pt" w="300pt" h="300pt"/><medium short="300pt" long="300pt"/></pageArea></pageSet>"#;

    #[test]
    fn flowed_subforms_overflow_onto_new_pages() {
        let template = parse(&format!(
            r#"<template><subform name="f" layout="tb">{PAGE_SET}
            <subform name="item" layout="tb"><occur min="0" max="-1"/>
              <field name="v" w="100pt" h="30pt"/></subform></subform></template>"#
        ));
        let data = parse(
            r#"<data><f><item><v>1</v></item><item><v>2</v></item><item><v>3</v></item><item><v>4</v></item><item><v>5</v></item><item><v>6</v></item><item><v>7</v></item><item><v>8</v></item></f></data>"#,
        );
        let tpl = Template::new(&template);
        let model = build_form_model(&tpl, &template, Some(&data)).unwrap();
        let mut book = FontBook::new(None, Vec::new());
        let pages = layout_form(&tpl, &model, &mut book);
        // 8 fields x 30pt in 200pt content areas: 6 on page 1, 2 on page 2.
        assert_eq!(pages.len(), 2);
        let fields = |page: &LayoutPage| {
            page.items
                .iter()
                .filter(|item| item.node.kind == NodeKind::Field)
                .count()
        };
        assert_eq!(fields(&pages[0]), 6);
        assert_eq!(fields(&pages[1]), 2);
        let first = pages[0]
            .items
            .iter()
            .find(|item| item.node.kind == NodeKind::Field)
            .unwrap();
        assert_eq!((first.x, first.y), (36.0, 36.0));
        // orderedOccurrence with unlimited occurrences reuses page area 1.
        assert_eq!(pages[1].page_area, 0);
    }

    #[test]
    fn break_before_targets_page_area() {
        let template = parse(&format!(
            r#"<template><subform name="f" layout="tb">{PAGE_SET}
            <subform name="a" w="100pt" h="20pt"/>
            <subform name="b" w="100pt" h="20pt"><breakBefore targetType="pageArea" target="P2"/></subform>
            <subform name="c" w="100pt" h="20pt"><break before="contentArea"/></subform>
            </subform></template>"#
        ));
        let tpl = Template::new(&template);
        let model = build_form_model(&tpl, &template, None).unwrap();
        let mut book = FontBook::new(None, Vec::new());
        let pages = layout_form(&tpl, &model, &mut book);
        assert_eq!(pages.len(), 3);
        assert_eq!(pages[1].page_area, 1);
        assert_eq!((pages[1].width, pages[1].height), (300.0, 300.0));
    }

    #[test]
    fn positioned_and_lr_tb_children() {
        let template = parse(
            r#"<template><subform name="f"><pageSet><pageArea><contentArea x="10pt" y="20pt" w="500pt" h="700pt"/></pageArea></pageSet>
            <subform name="box" x="5pt" y="6pt" layout="lr-tb" w="100pt">
              <field name="a" w="60pt" h="10pt"/><field name="b" w="60pt" h="12pt"/>
            </subform>
            <draw name="d" x="200pt" y="100pt" w="40pt" h="20pt" anchorType="middleCenter"/>
            </subform></template>"#,
        );
        let tpl = Template::new(&template);
        let model = build_form_model(&tpl, &template, None).unwrap();
        let mut book = FontBook::new(None, Vec::new());
        let pages = layout_form(&tpl, &model, &mut book);
        assert_eq!(pages.len(), 1);
        let find = |name: &str| {
            pages[0]
                .items
                .iter()
                .find(|item| item.node.name() == name)
                .unwrap()
        };
        let a = find("a");
        let b = find("b");
        assert_eq!((a.x, a.y), (15.0, 26.0));
        // b wraps below a (60 + 60 > 100).
        assert_eq!((b.x, b.y), (15.0, 36.0));
        assert_eq!(find("box").h, 22.0);
        let d = find("d");
        assert_eq!((d.x, d.y), (190.0, 110.0));
    }

    #[test]
    fn table_rows_use_column_widths() {
        let template = parse(
            r#"<template><subform name="f" layout="tb"><pageSet><pageArea><contentArea w="500pt" h="700pt"/></pageArea></pageSet>
            <subform name="t" layout="table" columnWidths="50pt 70pt">
              <subform name="r" layout="row"><draw name="c1" h="10pt"/><draw name="c2" h="15pt"/></subform>
            </subform></subform></template>"#,
        );
        let tpl = Template::new(&template);
        let model = build_form_model(&tpl, &template, None).unwrap();
        let mut book = FontBook::new(None, Vec::new());
        let pages = layout_form(&tpl, &model, &mut book);
        let find = |name: &str| {
            pages[0]
                .items
                .iter()
                .find(|item| item.node.name() == name)
                .unwrap()
        };
        assert_eq!((find("c1").w, find("c1").h), (50.0, 15.0));
        assert_eq!((find("c2").x, find("c2").w), (50.0, 70.0));
    }
}
