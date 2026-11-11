//! XFA Scripting Object Model (PDFium `fxjs/xfa`, `CFXJSE_Engine`,
//! `CFXJSE_ResolveProcessor`), over an owned copy of the merged form.
//!
//! Scripts never touch the form model directly: the event runner copies
//! the merged form into this arena ([`Dom`]), FormCalc and JavaScript read
//! and write it through [`Dom::get`], [`Dom::set`] and [`Dom::call`], and
//! the runner writes the changes (values, presence, captions, choice
//! items, colours, instance counts) back before layout.
//!
//! SOM expressions: `a.b[2].c`, `a[*]`, `a[+1]`/`a[-1]`, `..name`
//! (descendants), `#class`, `*`, `.[FormCalc predicate]`, `$` (this),
//! `$form`, `$host`, `$layout`, `$event`, `$data`, `$record`, `!`
//! (`xfa.datasets`), `xfa.*`, and unqualified names resolved by XFA
//! scoping (children, then ancestors' children, with unnamed containers
//! transparent and instance-index inference).
//!
//! The data DOM (PDFium `CXFA_DocumentBuilder::ParseDataGroup`) lives in
//! the same arena: `xfa.datasets` (a `dataModel`) holds the datasets
//! packet's `dataGroup` and `dataValue` nodes, attributes become
//! `metaData` data values, and fields, exclusion groups and subforms
//! carry the data node they are bound to. Writing a bound field's value
//! updates its data node and every other field bound to it, and writing a
//! data value updates the fields bound to it (PDFium `SyncValue`).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use super::locale::LocaleSet;
use super::picture;
use super::RuntimeInstant;

/// Container classes of the script DOM.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Class {
    Subform,
    Area,
    ExclGroup,
    Field,
    Draw,
    PageSet,
    PageArea,
    /// `<variables><script name=...>` script object.
    ScriptObject,
    /// `<variables><text name=...>` (and other value) variables.
    Variable,
    /// `xfa.datasets`.
    DataModel,
    /// A data element with element children (`xfa.datasets.data` too).
    DataGroup,
    /// A data element with character data, or an attribute (`metaData`).
    DataValue,
}

impl Class {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Class::Subform => "subform",
            Class::Area => "area",
            Class::ExclGroup => "exclGroup",
            Class::Field => "field",
            Class::Draw => "draw",
            Class::PageSet => "pageSet",
            Class::PageArea => "pageArea",
            Class::ScriptObject => "script",
            Class::Variable => "text",
            Class::DataModel => "dataModel",
            Class::DataGroup => "dataGroup",
            Class::DataValue => "dataValue",
        }
    }

    fn container(self) -> bool {
        !matches!(self, Class::ScriptObject | Class::Variable) && !self.is_data()
    }

    pub(crate) fn is_data(self) -> bool {
        matches!(self, Class::DataModel | Class::DataGroup | Class::DataValue)
    }
}

/// A container's `<validate>` (XFA `nullTest`, `formatTest`,
/// `scriptTest`; PDFium `CXFA_Validate`).
#[derive(Clone, Debug)]
pub(crate) struct ValidateDef {
    /// `disabled` (default), `warning` or `error`.
    pub null_test: String,
    /// `warning` (default), `error` or `disabled`.
    pub format_test: String,
    /// `error` (default), `warning` or `disabled`.
    pub script_test: String,
    /// `<validate><picture>`.
    pub picture: Option<String>,
    pub script: Option<ScriptDef>,
    /// `<message><text name="nullTest|formatTest|scriptTest">`.
    pub null_message: Option<String>,
    pub format_message: Option<String>,
    pub script_message: Option<String>,
}

/// One failed validation test.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ValidationFailure {
    /// `nullTest`, `formatTest` or `scriptTest`.
    pub test: &'static str,
    /// `error` or `warning`.
    pub severity: String,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lang {
    FormCalc,
    JavaScript,
}

/// Script-triggering activities the renderer runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Activity {
    Initialize,
    Calculate,
    FormReady,
    LayoutReady,
    DocReady,
    /// Interactive events (click, change, ...): never run by the renderer
    /// but reachable through `execEvent`.
    Other,
}

#[derive(Clone, Debug)]
pub(crate) struct ScriptDef {
    pub activity: Activity,
    /// The `activity` attribute (for `execEvent`).
    pub activity_name: String,
    pub lang: Lang,
    pub source: String,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Changes {
    pub value: bool,
    pub presence: bool,
    pub caption: bool,
    pub items: bool,
    pub colors: bool,
}

impl Changes {
    pub(crate) fn any(&self) -> bool {
        self.value || self.presence || self.caption || self.items || self.colors
    }
}

/// One script DOM node.
#[derive(Clone, Debug)]
pub(crate) struct SNode {
    pub class: Class,
    pub name: String,
    /// Template element id (document order) or [`NO_TMPL`].
    pub tmpl: usize,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    /// Canonical raw value.
    pub value: Option<String>,
    /// `text`, `decimal`, `integer`, `float`, `date`, `time`, `dateTime`,
    /// `boolean`, `image`.
    pub value_kind: String,
    /// UI widget element name (`textEdit`, `numericEdit`, `checkButton`...).
    pub ui: String,
    pub presence: String,
    pub access: String,
    pub caption: Option<String>,
    pub caption_presence: Option<String>,
    /// Template `caption` attributes (`placement`, `reserve`).
    pub caption_placement: Option<String>,
    pub caption_reserve: Option<String>,
    pub has_caption: bool,
    /// Choice-list items (display, save).
    pub items: Vec<(String, String)>,
    /// Check button on/off values (`<items>`).
    pub on_value: Option<String>,
    pub off_value: Option<String>,
    pub picture: Option<String>,
    pub locale: Option<String>,
    pub fill_color: Option<String>,
    pub border_color: Option<String>,
    pub font_color: Option<String>,
    /// Template x/y/w/h attribute text.
    pub geometry: [Option<String>; 4],
    pub scripts: Vec<ScriptDef>,
    /// `occur` (min, max; max `None` = unbounded) of repeatable subforms.
    pub occur: Option<(usize, Option<usize>)>,
    /// Repeatable subform templates managed under this container (for
    /// instance managers with zero instances): (name, tmpl, min, max).
    pub managed: Vec<(String, usize, usize, Option<usize>)>,
    /// Page-area index (page areas) or other class-specific data.
    pub aux: usize,
    pub changes: Changes,
    /// The data node a field, exclusion group or subform is bound to.
    pub bind: Option<usize>,
    /// A data value made from an XML attribute (`contains="metaData"`).
    pub meta: bool,
    pub validate: Option<Box<ValidateDef>>,
}

pub(crate) const NO_TMPL: usize = usize::MAX;

impl SNode {
    pub(crate) fn new(class: Class, name: &str, tmpl: usize) -> Self {
        Self {
            class,
            name: name.to_string(),
            tmpl,
            parent: None,
            children: Vec::new(),
            value: None,
            value_kind: "text".to_string(),
            ui: String::new(),
            presence: "visible".to_string(),
            access: "open".to_string(),
            caption: None,
            caption_presence: None,
            caption_placement: None,
            caption_reserve: None,
            has_caption: false,
            items: Vec::new(),
            on_value: None,
            off_value: None,
            picture: None,
            locale: None,
            fill_color: None,
            border_color: None,
            font_color: None,
            geometry: [None, None, None, None],
            scripts: Vec::new(),
            occur: None,
            managed: Vec::new(),
            aux: 0,
            changes: Changes::default(),
            bind: None,
            meta: false,
            validate: None,
        }
    }

    fn numeric(&self) -> bool {
        matches!(self.value_kind.as_str(), "decimal" | "float" | "integer")
            || self.ui == "numericEdit"
    }
}

/// A scripting value shared by FormCalc and JavaScript.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Val {
    Null,
    Num(f64),
    Str(String),
    Bool(bool),
    Obj(Obj),
}

impl Val {
    pub(crate) fn to_text(&self) -> String {
        match self {
            Val::Null => String::new(),
            Val::Num(n) => format_number(*n),
            Val::Str(s) => s.clone(),
            Val::Bool(b) => if *b { "1" } else { "0" }.to_string(),
            Val::Obj(_) => String::new(),
        }
    }

    pub(crate) fn to_num(&self) -> f64 {
        match self {
            Val::Null | Val::Obj(_) => 0.0,
            Val::Num(n) => *n,
            Val::Bool(b) => f64::from(u8::from(*b)),
            Val::Str(s) => parse_leading_number(s),
        }
    }
}

/// FormCalc string-to-number: the leading numeric prefix, else 0.
pub(crate) fn parse_leading_number(text: &str) -> f64 {
    let text = text.trim();
    if let Ok(value) = text.parse::<f64>() {
        if value.is_finite() {
            return value;
        }
    }
    let bytes = text.as_bytes();
    let mut end = 0;
    let mut seen_digit = false;
    let mut seen_dot = false;
    let mut seen_exp = false;
    while end < bytes.len() {
        let b = bytes[end];
        let ok = match b {
            b'0'..=b'9' => {
                seen_digit = true;
                true
            }
            b'+' | b'-' => end == 0 || matches!(bytes[end - 1], b'e' | b'E'),
            b'.' if !seen_dot && !seen_exp => {
                seen_dot = true;
                true
            }
            b'e' | b'E' if seen_digit && !seen_exp => {
                seen_exp = true;
                true
            }
            _ => false,
        };
        if !ok {
            break;
        }
        end += 1;
    }
    let mut slice = &text[..end];
    while !slice.is_empty() {
        if let Ok(value) = slice.parse::<f64>() {
            return value;
        }
        slice = &slice[..slice.len() - 1];
    }
    0.0
}

/// Canonical number text: integers without a fraction, otherwise the
/// shortest round-trip representation (like JavaScript `String(n)` for
/// ordinary magnitudes).
pub(crate) fn format_number(value: f64) -> String {
    if !value.is_finite() {
        return if value.is_nan() {
            "NaN".to_string()
        } else if value > 0.0 {
            "Infinity".to_string()
        } else {
            "-Infinity".to_string()
        };
    }
    if value == value.trunc() && value.abs() < 1e15 {
        return format!("{}", value as i64);
    }
    let text = format!("{value}");
    if text.contains('e') {
        return text;
    }
    text
}

/// Script-visible objects.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Obj {
    Node(usize),
    Nodes(Vec<usize>),
    /// The live child list of a data node (`nodes`: `append`, `insert`,
    /// `remove`, `item`, `length`).
    Children(usize),
    Xfa,
    FormModel,
    Host,
    Layout,
    Event,
    Data,
    Template,
    /// Instance manager of the repeatable template `tmpl` under `parent`.
    Manager {
        parent: usize,
        tmpl: usize,
    },
    /// A property object of a node (`caption`, `caption.value`, `font`...).
    Prop {
        node: usize,
        path: Vec<String>,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct ScriptError(pub String);

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

pub(crate) type SResult<T> = Result<T, ScriptError>;

pub(crate) fn err<T>(message: impl Into<String>) -> SResult<T> {
    Err(ScriptError(message.into()))
}

/// Follow-up work requested by scripts, run by the event runner after the
/// current script returns.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Request {
    Event(usize, Activity, String),
    /// A new instance: run its initialize events.
    Instance(usize),
    RecalculateAll,
    /// `execValidate` / `execEvent("validate")`: validate the subtree.
    Validate(usize),
}

/// The `$event` (`xfa.event`) properties of the event being dispatched
/// (PDFium `CXFA_EventParam`); empty outside interactive events.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct EventState {
    pub name: String,
    pub change: String,
    pub prev_text: String,
    pub full_text: String,
    pub sel_start: i64,
    pub sel_end: i64,
    /// The node the event was dispatched to (`xfa.event.target`).
    pub target: Option<usize>,
    /// `xfa.event.cancelAction`, set by a script to veto the change.
    pub cancel_action: bool,
}

impl EventState {
    /// The text after the change: the selection of `prevText` replaced by
    /// `change` (PDFium `CXFA_EventParam::GetNewText`).
    pub(crate) fn new_text(&self) -> String {
        let chars: Vec<char> = self.prev_text.chars().collect();
        let start = (self.sel_start.max(0) as usize).min(chars.len());
        let end = (self.sel_end.max(0) as usize).clamp(start, chars.len());
        let mut out: String = chars[..start].iter().collect();
        out.push_str(&self.change);
        out.extend(chars[end..].iter());
        out
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct LayoutInfo {
    pub page_count: usize,
    /// 0-based first page of each laid-out node.
    pub node_pages: HashMap<usize, usize>,
    /// Layout extents (x, y, w, h in points) of laid-out nodes.
    pub node_rects: HashMap<usize, [f64; 4]>,
}

/// Execution limits shared by FormCalc and JavaScript.
#[derive(Clone, Debug)]
pub(crate) struct Limits {
    pub max_runtime_ms: u64,
    pub max_ops: u64,
    pub max_call_depth: usize,
    pub max_string_bytes: usize,
    pub max_nodes: usize,
    pub max_mutations: usize,
    pub max_instances: usize,
    pub max_source_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_runtime_ms: 3_000,
            max_ops: 20_000_000,
            max_call_depth: 64,
            max_string_bytes: 1024 * 1024,
            max_nodes: 100_000,
            max_mutations: 200_000,
            max_instances: 1_024,
            max_source_bytes: 256 * 1024,
        }
    }
}

/// The script DOM.
pub(crate) struct Dom {
    pub nodes: Vec<SNode>,
    pub root: usize,
    /// Detached instance prototypes by template id.
    pub protos: HashMap<usize, usize>,
    pub locales: LocaleSet,
    /// `app.alert` / `xfa.host.messageBox` output.
    pub log: Vec<String>,
    pub requests: Vec<Request>,
    pub layout: Option<LayoutInfo>,
    /// The page a master-page script is evaluated for (0-based).
    pub current_page: Option<usize>,
    pub limits: Limits,
    pub started: RuntimeInstant,
    pub ops: u64,
    pub mutations: usize,
    /// Set once a limit is hit: no further scripts run.
    pub exhausted: bool,
    /// Instance-structure changes (add/remove/move).
    pub structure_changed: bool,
    /// Value reads recorded while a calculate script runs (dependency
    /// tracking, as PDFium's calculate dependency lists).
    pub reads: RefCell<Option<HashSet<usize>>>,
    /// Nodes whose value changed since the runner last looked.
    pub changed: Vec<usize>,
    /// `xfa.datasets`, `xfa.datasets.data`.
    pub datasets: Option<usize>,
    pub data: Option<usize>,
    /// Data node -> the form nodes bound to it.
    pub bound: HashMap<usize, Vec<usize>>,
    /// `xfa.host.validationsEnabled`.
    pub validations_enabled: bool,
    /// Set once the initial validation pass ran (before it, `execValidate`
    /// skips the null and format tests, as PDFium's `kNeedsInitApp`).
    pub validate_ready: bool,
    /// Form nodes whose value changed since they were last validated.
    pub validate_dirty: HashSet<usize>,
    /// `$event` of the interactive event being dispatched.
    pub event: EventState,
    /// Qualified element names and namespace declarations of data nodes
    /// loaded from the datasets packet (for exporting it again).
    pub data_xml: HashMap<usize, (String, Vec<(String, String)>)>,
}

/// SOM index: none, `[n]`, `[+n]`/`[-n]`, `[*]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SomIndex {
    Default,
    At(i64),
    Rel(i64),
    All,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SomName {
    Name(String),
    Class(String),
    Star,
    /// `$`, `$form`, `$host`...: the name after `$` (empty for `$`).
    Dollar(String),
    /// `!` (datasets).
    Bang,
    /// `.[FormCalc expression]` filter.
    Pred(String),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Seg {
    /// Preceded by `..` (descendant search).
    pub desc: bool,
    pub name: SomName,
    pub index: SomIndex,
}

/// Parses a SOM expression string.
pub(crate) fn parse_som(text: &str) -> SResult<Vec<Seg>> {
    let chars: Vec<char> = text.trim().chars().collect();
    let mut pos = 0;
    let mut segs = Vec::new();
    let mut desc = false;
    let mut first = true;
    while pos < chars.len() {
        let ch = chars[pos];
        if ch == '.' {
            if chars.get(pos + 1) == Some(&'.') {
                desc = true;
                pos += 2;
            } else {
                pos += 1;
            }
            if chars.get(pos) == Some(&'[') {
                // `.[predicate]`
                let start = pos + 1;
                let end = matching(&chars, pos, '[', ']')
                    .ok_or_else(|| ScriptError(format!("unterminated SOM predicate in {text}")))?;
                segs.push(Seg {
                    desc,
                    name: SomName::Pred(chars[start..end].iter().collect()),
                    index: SomIndex::Default,
                });
                desc = false;
                pos = end + 1;
            }
            continue;
        }
        let name = if ch == '$' {
            pos += 1;
            let start = pos;
            while pos < chars.len() && is_name_char(chars[pos]) {
                pos += 1;
            }
            SomName::Dollar(chars[start..pos].iter().collect())
        } else if ch == '!' && first {
            pos += 1;
            SomName::Bang
        } else if ch == '#' {
            pos += 1;
            let start = pos;
            while pos < chars.len() && is_name_char(chars[pos]) {
                pos += 1;
            }
            SomName::Class(chars[start..pos].iter().collect())
        } else if ch == '*' {
            pos += 1;
            SomName::Star
        } else if is_name_char(ch) {
            let start = pos;
            while pos < chars.len() && is_name_char(chars[pos]) {
                pos += 1;
            }
            SomName::Name(chars[start..pos].iter().collect())
        } else {
            return err(format!("unexpected '{ch}' in SOM expression {text}"));
        };
        let mut index = SomIndex::Default;
        if chars.get(pos) == Some(&'[') {
            let end = matching(&chars, pos, '[', ']')
                .ok_or_else(|| ScriptError(format!("unterminated SOM index in {text}")))?;
            let inner: String = chars[pos + 1..end].iter().collect();
            let inner = inner.trim();
            index = if inner == "*" {
                SomIndex::All
            } else if let Some(rel) = inner.strip_prefix('+') {
                SomIndex::Rel(rel.trim().parse().unwrap_or(0))
            } else if inner.starts_with('-') {
                SomIndex::Rel(inner.parse().unwrap_or(0))
            } else {
                SomIndex::At(
                    inner
                        .parse()
                        .map_err(|_| ScriptError(format!("bad SOM index [{inner}] in {text}")))?,
                )
            };
            pos = end + 1;
        }
        segs.push(Seg { desc, name, index });
        desc = false;
        first = false;
        // A bare `!name` continues without a dot.
        if matches!(segs.last().map(|s| &s.name), Some(SomName::Bang)) {
            continue;
        }
    }
    Ok(segs)
}

fn matching(chars: &[char], open_at: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 0;
    let mut quoted = false;
    for (index, &ch) in chars.iter().enumerate().skip(open_at) {
        if ch == '"' {
            quoted = !quoted;
        }
        if quoted {
            continue;
        }
        if ch == open {
            depth += 1;
        } else if ch == close {
            depth -= 1;
            if depth == 0 {
                return Some(index);
            }
        }
    }
    None
}

pub(crate) fn is_name_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_' || ch == '-' || ch == ':'
}

impl Dom {
    pub(crate) fn new(limits: Limits, locales: LocaleSet) -> Self {
        Self {
            nodes: Vec::new(),
            root: 0,
            protos: HashMap::new(),
            locales,
            log: Vec::new(),
            requests: Vec::new(),
            layout: None,
            current_page: None,
            limits,
            started: RuntimeInstant::now(),
            ops: 0,
            mutations: 0,
            exhausted: false,
            structure_changed: false,
            reads: RefCell::new(None),
            changed: Vec::new(),
            datasets: None,
            data: None,
            bound: HashMap::new(),
            validations_enabled: true,
            validate_ready: false,
            validate_dirty: HashSet::new(),
            event: EventState::default(),
            data_xml: HashMap::new(),
        }
    }

    /// `$record`: the first data group of `xfa.datasets.data`.
    pub(crate) fn record(&self) -> Option<usize> {
        let data = self.data?;
        self.nodes[data]
            .children
            .iter()
            .copied()
            .find(|&child| self.nodes[child].class == Class::DataGroup)
    }

    /// The objects `$data`, `$record` and `$datasets` (or `xfa.data`...)
    /// name; `None` for names that are not data models.
    fn data_objs(&self, name: &str) -> Option<Vec<Obj>> {
        let node = match name {
            "data" => self.data,
            "record" => self.record(),
            "datasets" => self.datasets,
            _ => return None,
        };
        Some(node.map(Obj::Node).into_iter().collect())
    }

    pub(crate) fn add(&mut self, node: SNode, parent: Option<usize>) -> usize {
        let id = self.nodes.len();
        self.nodes.push(node);
        self.nodes[id].parent = parent;
        if let Some(parent) = parent {
            self.nodes[parent].children.push(id);
        }
        id
    }

    /// Counts one operation; errors when a limit is exceeded.
    pub(crate) fn tick(&mut self, cost: u64) -> SResult<()> {
        self.ops = self.ops.saturating_add(cost);
        if self.ops > self.limits.max_ops {
            self.exhausted = true;
            return err("XFA script operation budget exhausted");
        }
        if self.ops % 1024 < cost.max(1)
            && self.started.elapsed_millis() > u128::from(self.limits.max_runtime_ms)
        {
            self.exhausted = true;
            return err("XFA script time budget exhausted");
        }
        Ok(())
    }

    pub(crate) fn time_left(&self) -> bool {
        !self.exhausted && self.started.elapsed_millis() <= u128::from(self.limits.max_runtime_ms)
    }

    pub(crate) fn check_string(&self, text: &str) -> SResult<()> {
        if text.len() > self.limits.max_string_bytes {
            return err("XFA script string exceeds the size limit");
        }
        Ok(())
    }

    fn transparent(&self, id: usize) -> bool {
        let node = &self.nodes[id];
        node.class == Class::Area || (node.class.container() && node.name.is_empty())
    }

    /// Children named `name`, looking through transparent containers.
    pub(crate) fn children_named(&self, id: usize, name: &str) -> Vec<usize> {
        let mut out = Vec::new();
        self.collect_named(id, name, &mut out, 0);
        out
    }

    fn collect_named(&self, id: usize, name: &str, out: &mut Vec<usize>, depth: usize) {
        if depth > 64 {
            return;
        }
        for &child in &self.nodes[id].children {
            if self.nodes[child].name == name {
                out.push(child);
            } else if self.transparent(child) {
                self.collect_named(child, name, out, depth + 1);
            }
        }
    }

    fn children_class(&self, id: usize, class: &str) -> Vec<usize> {
        self.nodes[id]
            .children
            .iter()
            .copied()
            .filter(|&child| self.nodes[child].class.name() == class)
            .collect()
    }

    fn descendants_named(&self, id: usize, name: &str, out: &mut Vec<usize>) {
        let mut stack: Vec<usize> = self.nodes[id].children.iter().rev().copied().collect();
        while let Some(node) = stack.pop() {
            if out.len() > 100_000 {
                return;
            }
            if self.nodes[node].name == name {
                out.push(node);
            }
            stack.extend(self.nodes[node].children.iter().rev().copied());
        }
    }

    fn is_ancestor_or_self(&self, ancestor: usize, mut node: usize) -> bool {
        for _ in 0..256 {
            if node == ancestor {
                return true;
            }
            match self.nodes[node].parent {
                Some(parent) => node = parent,
                None => return false,
            }
        }
        false
    }

    /// Picks from a same-name group by index (index inference for
    /// unindexed names: the member that contains `ctx`, else the first).
    fn select(&self, group: Vec<usize>, index: SomIndex, ctx: Option<usize>) -> Vec<usize> {
        match index {
            SomIndex::All => group,
            SomIndex::At(i) => usize::try_from(i)
                .ok()
                .and_then(|i| group.get(i).copied())
                .into_iter()
                .collect(),
            SomIndex::Default => {
                if let Some(ctx) = ctx {
                    if let Some(found) = group
                        .iter()
                        .copied()
                        .find(|&member| self.is_ancestor_or_self(member, ctx))
                    {
                        return vec![found];
                    }
                }
                group.into_iter().take(1).collect()
            }
            SomIndex::Rel(delta) => {
                let Some(ctx) = ctx else {
                    return Vec::new();
                };
                let Some(position) = group
                    .iter()
                    .position(|&member| self.is_ancestor_or_self(member, ctx))
                else {
                    return Vec::new();
                };
                let target = position as i64 + delta;
                usize::try_from(target)
                    .ok()
                    .and_then(|t| group.get(t).copied())
                    .into_iter()
                    .collect()
            }
        }
    }

    /// XFA scoping for an unqualified first name: the nearest container
    /// (from `from` upwards) with a child of that name, or a same-named
    /// ancestor's sibling group.
    pub(crate) fn scope_find(&self, from: usize, name: &str) -> Vec<usize> {
        let mut current = Some(from);
        let mut guard = 0;
        while let Some(id) = current {
            guard += 1;
            if guard > 256 {
                break;
            }
            let group = self.children_named(id, name);
            if !group.is_empty() {
                return group;
            }
            if self.nodes[id].name == name {
                if let Some(parent) = self.nodes[id].parent {
                    let group = self.children_named(parent, name);
                    if !group.is_empty() {
                        return group;
                    }
                }
                return vec![id];
            }
            current = self.nodes[id].parent;
        }
        Vec::new()
    }

    /// Instance manager by `_name` scoping.
    fn find_manager(&self, from: usize, name: &str) -> Option<Obj> {
        let group = self.scope_find(from, name);
        if let Some(&first) = group.first() {
            if self.nodes[first].class == Class::Subform {
                let parent = self.nodes[first].parent?;
                return Some(Obj::Manager {
                    parent,
                    tmpl: self.nodes[first].tmpl,
                });
            }
        }
        let mut current = Some(from);
        while let Some(id) = current {
            if let Some((_, tmpl, _, _)) = self.nodes[id].managed.iter().find(|m| m.0 == name) {
                return Some(Obj::Manager {
                    parent: id,
                    tmpl: *tmpl,
                });
            }
            current = self.nodes[id].parent;
        }
        None
    }

    /// Resolves a SOM string relative to `this` (or the form root).
    pub(crate) fn resolve(&mut self, this: Option<usize>, som: &str) -> SResult<Vec<Obj>> {
        self.tick(4)?;
        let segs = parse_som(som)?;
        let Some((first, rest)) = segs.split_first() else {
            return Ok(Vec::new());
        };
        let mut current = self.start(this, first)?;
        for seg in rest {
            if current.is_empty() {
                break;
            }
            current = self.step(&current, seg, this)?;
        }
        Ok(current)
    }

    /// The objects of a SOM expression's first segment.
    pub(crate) fn start(&mut self, this: Option<usize>, seg: &Seg) -> SResult<Vec<Obj>> {
        let objs = match &seg.name {
            SomName::Dollar(name) => match name.as_str() {
                "" => match this {
                    Some(id) => vec![Obj::Node(id)],
                    None => vec![Obj::FormModel],
                },
                "form" => vec![Obj::FormModel],
                "host" => vec![Obj::Host],
                "layout" => vec![Obj::Layout],
                "event" => vec![Obj::Event],
                "template" => vec![Obj::Template],
                "data" | "record" | "datasets" => self.data_objs(name).unwrap_or_default(),
                "connectionSet" | "config" => vec![Obj::Data],
                "xfa" => vec![Obj::Xfa],
                _ => Vec::new(),
            },
            // `!name`: a child of `xfa.datasets`.
            SomName::Bang => self.data_objs("datasets").unwrap_or_default(),
            SomName::Name(name) if name == "xfa" => vec![Obj::Xfa],
            SomName::Name(name) => {
                if let Some(manager_name) = name.strip_prefix('_') {
                    if let Some(manager) =
                        self.find_manager(this.unwrap_or(self.root), manager_name)
                    {
                        return Ok(vec![manager]);
                    }
                }
                let from = this.unwrap_or(self.root);
                let mut group = self.scope_find(from, name);
                if group.is_empty() && self.nodes[self.root].name == *name {
                    group = vec![self.root];
                }
                self.select(group, seg.index, this)
                    .into_iter()
                    .map(Obj::Node)
                    .collect()
            }
            SomName::Class(_) | SomName::Star | SomName::Pred(_) => {
                let base = vec![match this {
                    Some(id) => Obj::Node(id),
                    None => Obj::FormModel,
                }];
                return self.step(&base, seg, this);
            }
        };
        if seg.desc {
            let base = objs;
            let seg = Seg {
                desc: true,
                ..seg.clone()
            };
            return self.step(&base, &seg, this);
        }
        Ok(objs)
    }

    /// One SOM step from `current` objects (child names only; properties
    /// are handled by [`Dom::get`]).
    pub(crate) fn step(
        &mut self,
        current: &[Obj],
        seg: &Seg,
        ctx: Option<usize>,
    ) -> SResult<Vec<Obj>> {
        self.tick(1)?;
        let mut out = Vec::new();
        for obj in current {
            match obj {
                Obj::Xfa => {
                    if let SomName::Name(name) = &seg.name {
                        match name.as_str() {
                            "form" => out.push(Obj::FormModel),
                            "host" => out.push(Obj::Host),
                            "layout" => out.push(Obj::Layout),
                            "event" => out.push(Obj::Event),
                            "template" => out.push(Obj::Template),
                            "datasets" | "data" | "record" => {
                                out.extend(self.data_objs(name).unwrap_or_default());
                            }
                            "config" | "connectionSet" => out.push(Obj::Data),
                            _ if seg.desc => {
                                let mut found = Vec::new();
                                if self.nodes[self.root].name == *name {
                                    found.push(self.root);
                                }
                                self.descendants_named(self.root, name, &mut found);
                                out.extend(
                                    self.select(found, seg.index, ctx)
                                        .into_iter()
                                        .map(Obj::Node),
                                );
                            }
                            _ => {}
                        }
                    }
                }
                Obj::FormModel => {
                    let root = self.root;
                    let group = match &seg.name {
                        SomName::Name(name) if seg.desc => {
                            let mut found = Vec::new();
                            if self.nodes[root].name == *name {
                                found.push(root);
                            }
                            self.descendants_named(root, name, &mut found);
                            found
                        }
                        SomName::Name(name) if self.nodes[root].name == *name => vec![root],
                        SomName::Name(_) => Vec::new(),
                        SomName::Class(class) if class == "subform" => vec![root],
                        SomName::Star => vec![root],
                        _ => Vec::new(),
                    };
                    out.extend(
                        self.select(group, seg.index, ctx)
                            .into_iter()
                            .map(Obj::Node),
                    );
                }
                Obj::Node(id) => {
                    let id = *id;
                    let group = match &seg.name {
                        SomName::Name(name) if seg.desc => {
                            let mut found = Vec::new();
                            self.descendants_named(id, name, &mut found);
                            found
                        }
                        SomName::Name(name) => self.children_named(id, name),
                        SomName::Class(class)
                            if !seg.desc
                                && PROPERTY_CLASSES.contains(&class.as_str())
                                && self.children_class(id, class).is_empty() =>
                        {
                            // `resolveNode("#caption")`: a property object.
                            out.push(Obj::Prop {
                                node: id,
                                path: vec![class.clone()],
                            });
                            continue;
                        }
                        SomName::Class(class) => {
                            if seg.desc {
                                let mut found = Vec::new();
                                let mut stack: Vec<usize> =
                                    self.nodes[id].children.iter().rev().copied().collect();
                                while let Some(node) = stack.pop() {
                                    if self.nodes[node].class.name() == class {
                                        found.push(node);
                                    }
                                    stack.extend(self.nodes[node].children.iter().rev().copied());
                                }
                                found
                            } else {
                                self.children_class(id, class)
                            }
                        }
                        SomName::Star => {
                            let all: Vec<usize> = self.nodes[id].children.clone();
                            if seg.index == SomIndex::Default {
                                out.extend(all.into_iter().map(Obj::Node));
                                continue;
                            }
                            all
                        }
                        SomName::Pred(expr) => {
                            if super::formcalc::predicate(self, id, expr)? {
                                out.push(Obj::Node(id));
                            }
                            continue;
                        }
                        SomName::Dollar(_) | SomName::Bang => Vec::new(),
                    };
                    out.extend(
                        self.select(group, seg.index, ctx)
                            .into_iter()
                            .map(Obj::Node),
                    );
                }
                Obj::Nodes(ids) => {
                    let objs: Vec<Obj> = ids.iter().map(|id| Obj::Node(*id)).collect();
                    out.extend(self.step(&objs, seg, ctx)?);
                }
                Obj::Children(parent) => {
                    let objs: Vec<Obj> = self.nodes[*parent]
                        .children
                        .iter()
                        .map(|id| Obj::Node(*id))
                        .collect();
                    out.extend(self.step(&objs, seg, ctx)?);
                }
                _ => {}
            }
        }
        Ok(out)
    }

    /// Typed raw value of a node.
    pub(crate) fn raw_value(&self, id: usize) -> Val {
        if let Some(reads) = self.reads.borrow_mut().as_mut() {
            reads.insert(id);
            if let Some(parent) = self.nodes[id].parent {
                // Radio buttons read their group's value.
                if self.nodes[parent].class == Class::ExclGroup {
                    reads.insert(parent);
                }
            }
        }
        let node = &self.nodes[id];
        if node.class == Class::Field {
            if let Some(group) = node
                .parent
                .filter(|p| self.nodes[*p].class == Class::ExclGroup)
            {
                // Radio button: on when the group's value is its on value.
                let group_value = self.nodes[group].value.clone().unwrap_or_default();
                let on = node.on_value.clone().unwrap_or_else(|| "1".to_string());
                let text = if !group_value.is_empty() && group_value == on {
                    Some(on)
                } else {
                    node.off_value.clone().or(Some(String::new()))
                };
                return typed(text.as_deref(), node.numeric() || node.ui == "checkButton");
            }
        }
        if matches!(
            node.class,
            Class::Subform
                | Class::PageSet
                | Class::PageArea
                | Class::Area
                | Class::DataModel
                | Class::DataGroup
        ) {
            return Val::Null;
        }
        typed(node.value.as_deref(), node.numeric())
    }

    /// Default value of an object in expression context.
    pub(crate) fn value_of(&mut self, obj: &Obj) -> SResult<Val> {
        Ok(match obj {
            Obj::Node(id) => self.raw_value(*id),
            Obj::Nodes(ids) if ids.len() == 1 => self.raw_value(ids[0]),
            Obj::Prop { node, path } => self.prop_value(*node, path),
            _ => Val::Null,
        })
    }

    fn prop_value(&self, node: usize, path: &[String]) -> Val {
        let joined = path.join(".");
        let n = &self.nodes[node];
        match joined.as_str() {
            p if p.starts_with("value") => self.raw_value(node),
            p if p.starts_with("caption") => n.caption.clone().map_or(Val::Null, Val::Str),
            "border.fill.color" | "fill.color" => {
                Val::Str(n.fill_color.clone().unwrap_or_default())
            }
            "font.fill.color" | "font.color" => Val::Str(n.font_color.clone().unwrap_or_default()),
            "border.edge.color" => Val::Str(n.border_color.clone().unwrap_or_default()),
            _ => Val::Null,
        }
    }

    /// Sets a node's raw value.
    pub(crate) fn set_value(&mut self, id: usize, value: &Val) -> SResult<()> {
        let text = match value {
            Val::Null => None,
            Val::Obj(obj) => {
                let inner = self.value_of(&obj.clone())?;
                if matches!(inner, Val::Null) {
                    None
                } else {
                    Some(inner.to_text())
                }
            }
            other => Some(other.to_text()),
        };
        if let Some(text) = &text {
            self.check_string(text)?;
        }
        let node = &self.nodes[id];
        match node.class {
            Class::Field => {
                if let Some(group) = node
                    .parent
                    .filter(|p| self.nodes[*p].class == Class::ExclGroup)
                {
                    let on = node.on_value.clone().unwrap_or_else(|| "1".to_string());
                    let is_on = text.as_deref() == Some(on.as_str());
                    let group_value = self.nodes[group].value.clone();
                    let new_group = if is_on {
                        Some(on)
                    } else if group_value.as_deref() == Some(on.as_str()) {
                        None
                    } else {
                        group_value.clone()
                    };
                    if new_group != group_value {
                        self.store_value(group, new_group)?;
                        self.sync_binding(group)?;
                    }
                    return Ok(());
                }
            }
            Class::ExclGroup | Class::Draw | Class::Variable | Class::DataValue => {}
            _ => return Ok(()),
        }
        let normalized = text.filter(|t| !t.is_empty() || self.nodes[id].value.is_some());
        if self.nodes[id].value != normalized {
            self.store_value(id, normalized)?;
            self.sync_binding(id)?;
        }
        Ok(())
    }

    /// Stores a raw value (no binding propagation).
    fn store_value(&mut self, id: usize, value: Option<String>) -> SResult<()> {
        self.mutate()?;
        let node = &mut self.nodes[id];
        node.value = value;
        if !node.class.is_data() {
            node.changes.value = true;
            if matches!(node.class, Class::Field | Class::ExclGroup) {
                self.validate_dirty.insert(id);
            }
        }
        self.changed.push(id);
        Ok(())
    }

    /// After `id`'s value changed: copies it to the data value it is bound
    /// to (or that it is) and to every other form node bound to that data
    /// value (PDFium `CJX_Object::SetContent` with `bSyncData`).
    fn sync_binding(&mut self, id: usize) -> SResult<()> {
        let data = if self.nodes[id].class == Class::DataValue {
            id
        } else {
            match self.nodes[id].bind {
                Some(data) if self.nodes[data].class == Class::DataValue => data,
                _ => return Ok(()),
            }
        };
        let value = self.nodes[id].value.clone();
        if data != id && self.nodes[data].value != value {
            self.store_value(data, value.clone())?;
        }
        let forms = self.bound.get(&data).cloned().unwrap_or_default();
        for form in forms {
            if form != id && self.nodes[form].value != value {
                self.store_value(form, value.clone())?;
            }
        }
        Ok(())
    }

    fn mutate(&mut self) -> SResult<()> {
        self.mutations += 1;
        if self.mutations > self.limits.max_mutations {
            self.exhausted = true;
            return err("XFA script mutation limit exceeded");
        }
        Ok(())
    }

    fn index_of(&self, id: usize) -> usize {
        let Some(parent) = self.nodes[id].parent else {
            return 0;
        };
        let name = &self.nodes[id].name;
        let tmpl = self.nodes[id].tmpl;
        self.nodes[parent]
            .children
            .iter()
            .filter(|&&child| self.nodes[child].name == *name && self.nodes[child].tmpl == tmpl)
            .position(|&child| child == id)
            .unwrap_or(0)
    }

    pub(crate) fn som_expression(&self, id: usize) -> String {
        let mut parts = Vec::new();
        let mut current = Some(id);
        let mut top = id;
        while let Some(node) = current {
            let n = &self.nodes[node];
            let name = if n.name.is_empty() {
                format!("#{}", n.class.name())
            } else {
                n.name.clone()
            };
            parts.push(format!("{name}[{}]", self.index_of(node)));
            top = node;
            current = n.parent;
            if parts.len() > 256 {
                break;
            }
        }
        if !self.nodes[top].class.is_data() {
            parts.push("form[0]".to_string());
        }
        parts.push("xfa[0]".to_string());
        parts.reverse();
        parts.join(".")
    }

    fn instances(&self, parent: usize, tmpl: usize) -> Vec<usize> {
        self.nodes[parent]
            .children
            .iter()
            .copied()
            .filter(|&child| {
                self.nodes[child].tmpl == tmpl && self.nodes[child].class == Class::Subform
            })
            .collect()
    }

    fn occur_of(&self, parent: usize, tmpl: usize) -> (usize, Option<usize>) {
        if let Some(&first) = self.instances(parent, tmpl).first() {
            if let Some(occur) = self.nodes[first].occur {
                return occur;
            }
        }
        self.nodes[parent]
            .managed
            .iter()
            .find(|m| m.1 == tmpl)
            .map_or((1, Some(1)), |m| (m.2, m.3))
    }

    fn manager_name(&self, parent: usize, tmpl: usize) -> String {
        self.instances(parent, tmpl)
            .first()
            .map(|&first| self.nodes[first].name.clone())
            .or_else(|| {
                self.nodes[parent]
                    .managed
                    .iter()
                    .find(|m| m.1 == tmpl)
                    .map(|m| m.0.clone())
            })
            .unwrap_or_default()
    }

    /// Deep-copies a (prototype) subtree under `parent` at `position`.
    fn clone_subtree(&mut self, source: usize, parent: usize, position: usize) -> SResult<usize> {
        if self.nodes.len() >= self.limits.max_nodes {
            self.exhausted = true;
            return err("XFA script node limit exceeded");
        }
        let mut copy = self.nodes[source].clone();
        copy.children = Vec::new();
        copy.parent = Some(parent);
        copy.changes = Changes::default();
        let id = self.nodes.len();
        self.nodes.push(copy);
        let position = position.min(self.nodes[parent].children.len());
        self.nodes[parent].children.insert(position, id);
        let children = self.nodes[source].children.clone();
        for (index, child) in children.into_iter().enumerate() {
            self.clone_subtree(child, id, index)?;
        }
        Ok(id)
    }

    fn insert_position(&self, parent: usize, tmpl: usize, instance: usize) -> usize {
        let children = &self.nodes[parent].children;
        let existing: Vec<usize> = children
            .iter()
            .enumerate()
            .filter(|(_, &c)| self.nodes[c].tmpl == tmpl)
            .map(|(i, _)| i)
            .collect();
        if let Some(&at) = existing.get(instance) {
            return at;
        }
        if let Some(&last) = existing.last() {
            return last + 1;
        }
        // No instances: before the first later template sibling.
        children
            .iter()
            .position(|&c| self.nodes[c].tmpl != NO_TMPL && self.nodes[c].tmpl > tmpl)
            .unwrap_or(children.len())
    }

    fn add_instance(&mut self, parent: usize, tmpl: usize, at: Option<usize>) -> SResult<Val> {
        let count = self.instances(parent, tmpl).len();
        let (_, max) = self.occur_of(parent, tmpl);
        if max.is_some_and(|max| count >= max) || count >= self.limits.max_instances {
            return err("The element [max] has violated its allowable number of occurrences.");
        }
        let Some(&proto) = self.protos.get(&tmpl) else {
            return err("no instance template");
        };
        let position = self.insert_position(parent, tmpl, at.unwrap_or(count));
        self.mutate()?;
        let id = self.clone_subtree(proto, parent, position)?;
        self.structure_changed = true;
        self.requests.push(Request::Instance(id));
        Ok(Val::Obj(Obj::Node(id)))
    }

    fn remove_instance(&mut self, parent: usize, tmpl: usize, index: usize) -> SResult<()> {
        let instances = self.instances(parent, tmpl);
        let (min, _) = self.occur_of(parent, tmpl);
        if instances.len() <= min {
            return err("The element [min] has violated its allowable number of occurrences.");
        }
        let Some(&target) = instances.get(index) else {
            return err("Index value is out of bounds.");
        };
        self.mutate()?;
        self.nodes[parent].children.retain(|&c| c != target);
        self.nodes[target].parent = None;
        self.structure_changed = true;
        Ok(())
    }

    fn set_instances(&mut self, parent: usize, tmpl: usize, wanted: &Val) -> SResult<()> {
        let wanted = match wanted {
            Val::Num(n) if n.is_finite() => *n,
            Val::Str(s) => s.trim().parse::<f64>().unwrap_or(-1.0),
            _ => -1.0,
        };
        let (min, max) = self.occur_of(parent, tmpl);
        if wanted < min as f64 {
            return err("The element [min] has violated its allowable number of occurrences.");
        }
        let mut wanted = wanted as usize;
        if let Some(max) = max {
            if wanted > max {
                return err("The element [max] has violated its allowable number of occurrences.");
            }
        }
        wanted = wanted.min(self.limits.max_instances);
        loop {
            let count = self.instances(parent, tmpl).len();
            if count < wanted {
                self.add_instance(parent, tmpl, None)?;
            } else if count > wanted {
                self.remove_instance(parent, tmpl, count - 1)?;
            } else {
                return Ok(());
            }
        }
    }

    fn move_instance(&mut self, parent: usize, tmpl: usize, from: usize, to: usize) -> SResult<()> {
        let instances = self.instances(parent, tmpl);
        let (Some(&node), Some(_)) = (instances.get(from), instances.get(to)) else {
            return err("Index value is out of bounds.");
        };
        self.mutate()?;
        let children = &mut self.nodes[parent].children;
        children.retain(|&c| c != node);
        let mut remaining: Vec<usize> = Vec::new();
        for (index, &child) in children.iter().enumerate() {
            if instances.contains(&child) {
                remaining.push(index);
            }
        }
        let position = remaining.get(to).copied().unwrap_or(children.len());
        children.insert(position, node);
        self.structure_changed = true;
        Ok(())
    }

    fn arg_num(args: &[Val], index: usize) -> f64 {
        args.get(index).map_or(0.0, Val::to_num)
    }

    fn arg_text(args: &[Val], index: usize) -> String {
        args.get(index).map(Val::to_text).unwrap_or_default()
    }

    /// Reads a property (or a child by name) of an object.
    pub(crate) fn get(&mut self, obj: &Obj, name: &str) -> SResult<Val> {
        self.tick(1)?;
        match obj {
            Obj::Node(id) => self.get_node(*id, name),
            Obj::Nodes(ids) => {
                if let Ok(index) = name.parse::<usize>() {
                    return Ok(ids
                        .get(index)
                        .map_or(Val::Null, |id| Val::Obj(Obj::Node(*id))));
                }
                match name {
                    "length" => Ok(Val::Num(ids.len() as f64)),
                    "className" => Ok(Val::Str("nodeList".into())),
                    _ => match ids.first() {
                        Some(&first) => self.get_node(first, name),
                        None => Ok(Val::Null),
                    },
                }
            }
            Obj::Children(parent) => {
                let ids = self.nodes[*parent].children.clone();
                if let Ok(index) = name.parse::<usize>() {
                    return Ok(ids
                        .get(index)
                        .map_or(Val::Null, |id| Val::Obj(Obj::Node(*id))));
                }
                Ok(match name {
                    "length" => Val::Num(ids.len() as f64),
                    "className" => Val::Str("nodeList".into()),
                    _ => Val::Null,
                })
            }
            Obj::Manager { parent, tmpl } => {
                let (parent, tmpl) = (*parent, *tmpl);
                let (min, max) = self.occur_of(parent, tmpl);
                Ok(match name {
                    "count" => Val::Num(self.instances(parent, tmpl).len() as f64),
                    "min" => Val::Num(min as f64),
                    "max" => Val::Num(max.map_or(-1.0, |m| m as f64)),
                    "name" => Val::Str(format!("_{}", self.manager_name(parent, tmpl))),
                    "className" => Val::Str("instanceManager".into()),
                    _ => Val::Null,
                })
            }
            Obj::Xfa => Ok(match name {
                "form" => Val::Obj(Obj::FormModel),
                "host" => Val::Obj(Obj::Host),
                "layout" => Val::Obj(Obj::Layout),
                "event" => Val::Obj(Obj::Event),
                "template" => Val::Obj(Obj::Template),
                "datasets" | "data" | "record" => self
                    .data_objs(name)
                    .and_then(|objs| objs.into_iter().next())
                    .map_or(Val::Null, Val::Obj),
                "config" | "connectionSet" => Val::Obj(Obj::Data),
                "className" => Val::Str("xfa".into()),
                "name" => Val::Str("xfa".into()),
                _ => Val::Null,
            }),
            Obj::FormModel => {
                let root = self.root;
                if self.nodes[root].name == name {
                    return Ok(Val::Obj(Obj::Node(root)));
                }
                Ok(match name {
                    "className" => Val::Str("form".into()),
                    "name" => Val::Str("form".into()),
                    _ => {
                        let group = self.children_named(root, name);
                        match group.first() {
                            Some(&first) => Val::Obj(Obj::Node(first)),
                            None => Val::Null,
                        }
                    }
                })
            }
            Obj::Host => Ok(match name {
                "name" => Val::Str("Acrobat".into()),
                "appType" => Val::Str("Exchange".into()),
                "version" => Val::Str("7.05".into()),
                "platform" => Val::Str("WIN".into()),
                "language" => Val::Str("ENU".into()),
                "variation" => Val::Str("Full".into()),
                "title" => Val::Str(String::new()),
                "numPages" => Val::Num(self.layout.as_ref().map_or(1, |l| l.page_count) as f64),
                "currentPage" => Val::Num(self.current_page.unwrap_or(0) as f64),
                "calculationsEnabled" => Val::Bool(true),
                "validationsEnabled" => Val::Bool(self.validations_enabled),
                "className" => Val::Str("hostPseudoModel".into()),
                _ => Val::Null,
            }),
            Obj::Layout => Ok(match name {
                "ready" => Val::Bool(self.layout.is_some()),
                "className" => Val::Str("layoutPseudoModel".into()),
                _ => Val::Null,
            }),
            Obj::Event => {
                let event = &self.event;
                Ok(match name {
                    "name" => Val::Str(event.name.clone()),
                    "change" => Val::Str(event.change.clone()),
                    "newText" => Val::Str(event.new_text()),
                    "prevText" => Val::Str(event.prev_text.clone()),
                    "fullText" => Val::Str(event.full_text.clone()),
                    "selStart" => Val::Num(event.sel_start as f64),
                    "selEnd" => Val::Num(event.sel_end as f64),
                    "cancelAction" => Val::Bool(event.cancel_action),
                    "modifier" | "shift" | "keyDown" | "reenter" => Val::Bool(false),
                    "commitKey" => Val::Num(0.0),
                    "target" => event.target.map_or(Val::Null, |id| Val::Obj(Obj::Node(id))),
                    "newContentType" | "prevContentType" | "soapFaultCode" | "soapFaultString" => {
                        Val::Str(String::new())
                    }
                    "className" => Val::Str("eventPseudoModel".into()),
                    _ => Val::Null,
                })
            }
            Obj::Data | Obj::Template => Ok(Val::Null),
            Obj::Prop { node, path } => self.get_prop(*node, path, name),
        }
    }

    fn get_node(&mut self, id: usize, name: &str) -> SResult<Val> {
        if self.nodes[id].class.is_data() {
            return Ok(self.get_data_node(id, name));
        }
        // `subform._row`: the instance manager of a child repeating subform.
        if let Some(rest) = name.strip_prefix('_').filter(|rest| !rest.is_empty()) {
            let group = self.children_named(id, rest);
            if let Some(&first) = group.first() {
                if self.nodes[first].class == Class::Subform {
                    if let Some(parent) = self.nodes[first].parent {
                        return Ok(Val::Obj(Obj::Manager {
                            parent,
                            tmpl: self.nodes[first].tmpl,
                        }));
                    }
                }
            }
            if let Some((_, tmpl, _, _)) = self.nodes[id].managed.iter().find(|m| m.0 == rest) {
                return Ok(Val::Obj(Obj::Manager {
                    parent: id,
                    tmpl: *tmpl,
                }));
            }
        }
        // Children by name take precedence over properties.
        if !matches!(
            name,
            "rawValue"
                | "presence"
                | "name"
                | "parent"
                | "className"
                | "index"
                | "value"
                | "caption"
                | "length"
        ) {
            let group = self.children_named(id, name);
            if let Some(&first) = group.first() {
                return Ok(Val::Obj(Obj::Node(first)));
            }
        }
        let node = &self.nodes[id];
        let prop = |path: &str| {
            Val::Obj(Obj::Prop {
                node: id,
                path: vec![path.to_string()],
            })
        };
        Ok(match name {
            "rawValue" | "editValue" => self.raw_value(id),
            "formattedValue" => {
                let raw = node.value.clone().unwrap_or_default();
                if raw.is_empty() {
                    Val::Str(String::new())
                } else {
                    Val::Str(picture::format_value_in(
                        &raw,
                        node.picture.as_deref(),
                        &node.value_kind,
                        &self.locales,
                        node.locale.as_deref(),
                    ))
                }
            }
            "isNull" => Val::Bool(node.value.as_deref().is_none_or(str::is_empty)),
            "presence" => Val::Str(node.presence.clone()),
            "access" => Val::Str(node.access.clone()),
            "name" => Val::Str(node.name.clone()),
            "className" => Val::Str(node.class.name().into()),
            "index" | "instanceIndex" => Val::Num(self.index_of(id) as f64),
            "parent" => node.parent.map_or(Val::Null, |p| Val::Obj(Obj::Node(p))),
            "somExpression" => Val::Str(self.som_expression(id)),
            "all" => {
                let group = match node.parent {
                    Some(parent) => {
                        let name = node.name.clone();
                        self.children_named(parent, &name)
                    }
                    None => vec![id],
                };
                Val::Obj(Obj::Nodes(group))
            }
            "nodes" => Val::Obj(Obj::Nodes(node.children.clone())),
            "instanceManager" => match node.parent {
                Some(parent) if node.class == Class::Subform => Val::Obj(Obj::Manager {
                    parent,
                    tmpl: node.tmpl,
                }),
                _ => Val::Null,
            },
            "length" => {
                if node.class == Class::Field && node.ui == "choiceList" {
                    Val::Num(node.items.len() as f64)
                } else {
                    Val::Num(1.0)
                }
            }
            "selectedIndex" => {
                let value = node.value.clone().unwrap_or_default();
                Val::Num(
                    node.items
                        .iter()
                        .position(|(_, save)| *save == value)
                        .map_or(-1.0, |i| i as f64),
                )
            }
            "locale" => Val::Str(
                node.locale
                    .clone()
                    .unwrap_or_else(|| self.locales.ambient().to_string()),
            ),
            "x" => Val::Str(node.geometry[0].clone().unwrap_or_else(|| "0in".into())),
            "y" => Val::Str(node.geometry[1].clone().unwrap_or_else(|| "0in".into())),
            "w" => Val::Str(node.geometry[2].clone().unwrap_or_default()),
            "h" => Val::Str(node.geometry[3].clone().unwrap_or_default()),
            "fillColor" => Val::Str(
                node.fill_color
                    .clone()
                    .unwrap_or_else(|| "255,255,255".into()),
            ),
            "borderColor" => Val::Str(node.border_color.clone().unwrap_or_else(|| "0,0,0".into())),
            "fontColor" => Val::Str(node.font_color.clone().unwrap_or_else(|| "0,0,0".into())),
            "value" | "caption" | "font" | "border" | "fill" | "ui" | "items" | "margin"
            | "para" | "format" | "assist" | "validate" | "calculate" | "bind" | "occur" => {
                prop(name)
            }
            "mandatory" => Val::Str(
                node.validate
                    .as_ref()
                    .map_or("disabled", |v| v.null_test.as_str())
                    .to_string(),
            ),
            "id" | "use" | "usehref" | "relevant" => Val::Str(String::new()),
            "dataNode" => node
                .bind
                .map_or(Val::Null, |data| Val::Obj(Obj::Node(data))),
            "pageSet" => {
                let group = self.children_class(id, "pageSet");
                group.first().map_or(Val::Null, |&p| Val::Obj(Obj::Node(p)))
            }
            "variables" => Val::Null,
            "isContainer" => Val::Bool(node.class.container()),
            _ => Val::Null,
        })
    }

    /// Properties of data nodes (PDFium `CJX_DataValue`, `CJX_Node`,
    /// `CJX_Tree`): children by name first, as SOM resolution does.
    fn get_data_node(&self, id: usize, name: &str) -> Val {
        if let Some(&child) = self.children_named(id, name).first() {
            return Val::Obj(Obj::Node(child));
        }
        let node = &self.nodes[id];
        match name {
            "value" if node.class == Class::DataValue => {
                self.note_read(id);
                Val::Str(node.value.clone().unwrap_or_default())
            }
            "isNull" => {
                self.note_read(id);
                Val::Bool(node.value.is_none())
            }
            "contentType" => Val::Str(String::new()),
            "contains" if node.class == Class::DataValue => {
                Val::Str(if node.meta { "metaData" } else { "data" }.to_string())
            }
            "name" => Val::Str(node.name.clone()),
            "className" => Val::Str(node.class.name().into()),
            "index" | "instanceIndex" => Val::Num(self.index_of(id) as f64),
            "classIndex" => Val::Num(node.parent.map_or(0, |parent| {
                self.nodes[parent]
                    .children
                    .iter()
                    .filter(|&&c| self.nodes[c].class == node.class)
                    .position(|&c| c == id)
                    .unwrap_or(0)
            }) as f64),
            "parent" => match node.parent {
                Some(parent) => Val::Obj(Obj::Node(parent)),
                None if node.class == Class::DataModel => Val::Obj(Obj::Xfa),
                None => Val::Null,
            },
            "somExpression" => Val::Str(self.som_expression(id)),
            "all" => Val::Obj(Obj::Nodes(match node.parent {
                Some(parent) => self.children_named(parent, &node.name),
                None => vec![id],
            })),
            "nodes" => Val::Obj(Obj::Children(id)),
            "model" => self.datasets.map_or(Val::Null, |d| Val::Obj(Obj::Node(d))),
            "isContainer" => Val::Bool(false),
            "ns" => Val::Str(String::new()),
            "length" => Val::Num(1.0),
            _ => Val::Null,
        }
    }

    /// Records a value read for calculate dependency tracking.
    fn note_read(&self, id: usize) {
        if let Some(reads) = self.reads.borrow_mut().as_mut() {
            reads.insert(id);
        }
    }

    /// The `metaData` value (XML attribute) `name` of a data node.
    fn meta_child(&self, id: usize, name: &str) -> Option<usize> {
        self.nodes[id]
            .children
            .iter()
            .copied()
            .find(|&c| self.nodes[c].meta && self.nodes[c].name == name)
    }

    /// Creates a detached data node (`createNode`, `setAttribute`).
    pub(crate) fn new_data_node(&mut self, class: Class, name: &str) -> SResult<usize> {
        if self.nodes.len() >= self.limits.max_nodes {
            self.exhausted = true;
            return err("XFA script node limit exceeded");
        }
        self.mutate()?;
        Ok(self.add(SNode::new(class, name, NO_TMPL), None))
    }

    /// Moves the data node `child` under `parent` at `position` (end when
    /// `None`); refuses form nodes and cycles.
    fn attach_data(&mut self, parent: usize, child: usize, position: Option<usize>) -> SResult<()> {
        if !self.nodes[parent].class.is_data() || !self.nodes[child].class.is_data() {
            return err("XFAObject.append: Invalid argument (not a data node).");
        }
        if self.is_ancestor_or_self(child, parent) {
            return err("The node would become its own ancestor.");
        }
        self.mutate()?;
        if let Some(old) = self.nodes[child].parent {
            self.nodes[old].children.retain(|&c| c != child);
        }
        let children = &mut self.nodes[parent].children;
        let position = position.unwrap_or(children.len()).min(children.len());
        children.insert(position, child);
        self.nodes[child].parent = Some(parent);
        Ok(())
    }

    /// `saveXML` of a data node: the data as XML text.
    fn data_xml(&self, id: usize, out: &mut String, depth: usize) {
        fn escape(text: &str, out: &mut String) {
            for ch in text.chars() {
                match ch {
                    '<' => out.push_str("&lt;"),
                    '>' => out.push_str("&gt;"),
                    '&' => out.push_str("&amp;"),
                    '"' => out.push_str("&quot;"),
                    _ => out.push(ch),
                }
            }
        }
        if depth > 64 || out.len() > self.limits.max_string_bytes {
            return;
        }
        let node = &self.nodes[id];
        let name = match node.class {
            Class::DataModel => "xfa:datasets",
            _ if node.name.is_empty() => {
                escape(node.value.as_deref().unwrap_or(""), out);
                return;
            }
            _ => node.name.as_str(),
        };
        out.push('<');
        out.push_str(name);
        for &child in &node.children {
            let child = &self.nodes[child];
            if child.meta {
                out.push(' ');
                out.push_str(&child.name);
                out.push_str("=\"");
                escape(child.value.as_deref().unwrap_or(""), out);
                out.push('"');
            }
        }
        if node.class == Class::DataModel {
            out.push_str(" xmlns:xfa=\"http://www.xfa.org/schema/xfa-data/1.0/\"");
        }
        out.push('>');
        if node.class == Class::DataValue {
            escape(node.value.as_deref().unwrap_or(""), out);
        } else {
            for &child in &node.children {
                if !self.nodes[child].meta {
                    self.data_xml(child, out, depth + 1);
                }
            }
        }
        out.push_str("</");
        out.push_str(name);
        out.push('>');
    }

    fn get_prop(&mut self, node: usize, path: &[String], name: &str) -> SResult<Val> {
        let mut extended = path.to_vec();
        let n = &self.nodes[node];
        let joined = path.join(".");
        // Terminal reads.
        match (joined.as_str(), name) {
            (p, "value") if p.starts_with("caption.value.") => {
                return Ok(n.caption.clone().map_or(Val::Null, Val::Str));
            }
            (p, "value") if p.starts_with("value.") => return Ok(self.raw_value(node)),
            ("caption", "presence") => {
                return Ok(Val::Str(
                    n.caption_presence
                        .clone()
                        .unwrap_or_else(|| "visible".into()),
                ));
            }
            ("caption", "placement") => {
                return Ok(Val::Str(
                    n.caption_placement.clone().unwrap_or_else(|| "left".into()),
                ));
            }
            ("caption", "reserve") => {
                return Ok(Val::Str(
                    n.caption_reserve.clone().unwrap_or_else(|| "0in".into()),
                ));
            }
            (p, "value") if p.ends_with(".color") || p == "fill.color" => {
                return Ok(self.prop_value(node, path));
            }
            ("border.fill" | "fill", "presence") | ("border", "presence") => {
                return Ok(Val::Str("visible".into()));
            }
            ("font", "typeface") => return Ok(Val::Str("Courier".into())),
            ("items", "length") => return Ok(Val::Num(n.items.len() as f64)),
            ("font" | "border" | "fill" | "caption" | "value", "className") => {
                return Ok(Val::Str(path.last().cloned().unwrap_or_default()));
            }
            _ => {}
        }
        extended.push(name.trim_start_matches('#').to_string());
        Ok(Val::Obj(Obj::Prop {
            node,
            path: extended,
        }))
    }

    /// Writes a property of an object (or a node's value by name).
    pub(crate) fn set(&mut self, obj: &Obj, name: &str, value: &Val) -> SResult<()> {
        self.tick(1)?;
        match obj {
            Obj::Node(id) => self.set_node(*id, name, value),
            Obj::Nodes(ids) => {
                if let Some(&first) = ids.first() {
                    self.set_node(first, name, value)?;
                }
                Ok(())
            }
            Obj::Manager { parent, tmpl } => {
                if name == "count" {
                    self.set_instances(*parent, *tmpl, value)
                } else {
                    err("Invalid property set operation.")
                }
            }
            Obj::Prop { node, path } => {
                let mut full = path.clone();
                full.push(name.trim_start_matches('#').to_string());
                self.set_prop(*node, &full, value)
            }
            Obj::Host => {
                if name == "validationsEnabled" {
                    self.validations_enabled = match value {
                        Val::Bool(b) => *b,
                        Val::Str(s) => !matches!(s.trim(), "" | "0" | "false"),
                        other => other.to_num() != 0.0,
                    };
                }
                Ok(())
            }
            Obj::Event => {
                // Scripts may rewrite the pending change (PDFium copies
                // `change`, the selection and `cancelAction` back).
                match name {
                    "change" => {
                        let text = value.to_text();
                        self.check_string(&text)?;
                        self.event.change = text;
                    }
                    "selStart" => self.event.sel_start = value.to_num() as i64,
                    "selEnd" => self.event.sel_end = value.to_num() as i64,
                    "cancelAction" => {
                        self.event.cancel_action = match value {
                            Val::Bool(b) => *b,
                            Val::Str(s) => !matches!(s.trim(), "" | "0" | "false"),
                            other => other.to_num() != 0.0,
                        }
                    }
                    _ => {}
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn set_node(&mut self, id: usize, name: &str, value: &Val) -> SResult<()> {
        if self.nodes[id].class.is_data() {
            if let Some(&child) = self.children_named(id, name).first() {
                return self.set_value(child, value);
            }
            return match name {
                "value" => self.set_value(id, value),
                "isNull" if value.to_num() != 0.0 => {
                    if self.nodes[id].class == Class::DataValue && self.nodes[id].value.is_some() {
                        self.store_value(id, None)?;
                        self.sync_binding(id)?;
                    }
                    Ok(())
                }
                _ => Ok(()),
            };
        }
        match name {
            "rawValue" | "editValue" | "formattedValue" | "value" => self.set_value(id, value),
            "presence" => {
                let text = value.to_text();
                if !matches!(
                    text.as_str(),
                    "visible" | "invisible" | "hidden" | "inactive"
                ) {
                    return Ok(());
                }
                if self.nodes[id].presence != text {
                    self.mutate()?;
                    self.nodes[id].presence = text;
                    self.nodes[id].changes.presence = true;
                }
                Ok(())
            }
            "access" => {
                self.nodes[id].access = value.to_text();
                Ok(())
            }
            "fillColor" | "borderColor" | "fontColor" => {
                let text = value.to_text();
                self.mutate()?;
                let node = &mut self.nodes[id];
                match name {
                    "fillColor" => node.fill_color = Some(text),
                    "borderColor" => node.border_color = Some(text),
                    _ => node.font_color = Some(text),
                }
                node.changes.colors = true;
                Ok(())
            }
            "selectedIndex" => {
                let index = value.to_num();
                let save = usize::try_from(index as i64)
                    .ok()
                    .and_then(|i| self.nodes[id].items.get(i))
                    .map(|(_, save)| save.clone());
                self.set_value(id, &save.map_or(Val::Null, Val::Str))
            }
            _ => {
                // `field1.child = x` where child is a named child: its value.
                let group = self.children_named(id, name);
                if let Some(&first) = group.first() {
                    return self.set_value(first, value);
                }
                Ok(())
            }
        }
    }

    fn set_prop(&mut self, node: usize, path: &[String], value: &Val) -> SResult<()> {
        let joined = path.join(".");
        match joined.as_str() {
            p if p.starts_with("caption.value") => {
                let text = value.to_text();
                self.check_string(&text)?;
                if self.nodes[node].caption.as_deref() != Some(text.as_str()) {
                    self.mutate()?;
                    self.nodes[node].caption = Some(text);
                    self.nodes[node].changes.caption = true;
                }
                Ok(())
            }
            "caption.presence" => {
                self.mutate()?;
                self.nodes[node].caption_presence = Some(value.to_text());
                self.nodes[node].changes.caption = true;
                Ok(())
            }
            p if p.starts_with("value") => self.set_value(node, value),
            "border.fill.color.value" | "fill.color.value" => {
                self.mutate()?;
                self.nodes[node].fill_color = Some(value.to_text());
                self.nodes[node].changes.colors = true;
                Ok(())
            }
            "font.fill.color.value" | "font.color.value" => {
                self.mutate()?;
                self.nodes[node].font_color = Some(value.to_text());
                self.nodes[node].changes.colors = true;
                Ok(())
            }
            "border.edge.color.value" => {
                self.mutate()?;
                self.nodes[node].border_color = Some(value.to_text());
                self.nodes[node].changes.colors = true;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Calls a method of an object.
    pub(crate) fn call(
        &mut self,
        obj: &Obj,
        this: Option<usize>,
        method: &str,
        args: &[Val],
    ) -> SResult<Val> {
        self.tick(4)?;
        match obj {
            Obj::Xfa | Obj::FormModel => match method {
                "resolveNode" | "resolveNodes" => {
                    let som = Self::arg_text(args, 0);
                    let mut found = self.resolve_from_root(&som)?;
                    Ok(list_or_single(method == "resolveNodes", &mut found))
                }
                "recalculate" | "execCalculate" => {
                    self.requests.push(Request::RecalculateAll);
                    Ok(Val::Null)
                }
                "execInitialize" => {
                    let root = self.root;
                    self.requests.push(Request::Event(
                        root,
                        Activity::Initialize,
                        "initialize".into(),
                    ));
                    Ok(Val::Null)
                }
                "execValidate" => {
                    let root = self.root;
                    self.exec_validate(root, args)
                }
                "remerge" | "execEvent" | "createNode" => Ok(Val::Null),
                "isCompatibleNS" => Ok(Val::Bool(true)),
                _ => err(format!("unknown method {method}")),
            },
            Obj::Host => match method {
                "messageBox" => {
                    let message = Self::arg_text(args, 0);
                    self.log.push(message);
                    Ok(Val::Num(1.0))
                }
                "setFocus" | "beep" | "resetData" | "print" | "exportData" | "importData"
                | "gotoURL" | "openList" | "response" => Ok(Val::Null),
                "getFocus" => Ok(Val::Null),
                _ => err(format!("unknown method {method}")),
            },
            Obj::Layout => self.call_layout(method, args, this),
            Obj::Manager { parent, tmpl } => {
                let (parent, tmpl) = (*parent, *tmpl);
                match method {
                    "setInstances" => {
                        if args.len() != 1 {
                            return err("XFAObject.setInstances: Incorrect number of parameters passed to function.");
                        }
                        self.set_instances(parent, tmpl, &args[0])?;
                        Ok(Val::Null)
                    }
                    "addInstance" => {
                        if args.len() > 1 {
                            return err("XFAObject.addInstance: Incorrect number of parameters passed to function.");
                        }
                        self.add_instance(parent, tmpl, None)
                    }
                    "insertInstance" => {
                        if args.is_empty() || args.len() > 2 {
                            return err("XFAObject.insertInstance: Incorrect number of parameters passed to function.");
                        }
                        let index = Self::arg_num(args, 0).max(0.0) as usize;
                        let count = self.instances(parent, tmpl).len();
                        self.add_instance(parent, tmpl, Some(index.min(count)))
                    }
                    "removeInstance" => {
                        if args.len() != 1 {
                            return err("XFAObject.removeInstance: Incorrect number of parameters passed to function.");
                        }
                        let index = Self::arg_num(args, 0).max(0.0) as usize;
                        self.remove_instance(parent, tmpl, index)?;
                        Ok(Val::Null)
                    }
                    "moveInstance" => {
                        if args.len() != 2 {
                            return err("XFAObject.moveInstance: Incorrect number of parameters passed to function.");
                        }
                        let from = Self::arg_num(args, 0).max(0.0) as usize;
                        let to = Self::arg_num(args, 1).max(0.0) as usize;
                        self.move_instance(parent, tmpl, from, to)?;
                        Ok(Val::Null)
                    }
                    _ => err(format!("unknown method {method}")),
                }
            }
            Obj::Nodes(ids) => match method {
                "item" => {
                    let index = Self::arg_num(args, 0);
                    Ok(usize::try_from(index as i64)
                        .ok()
                        .and_then(|i| ids.get(i))
                        .map_or(Val::Null, |id| Val::Obj(Obj::Node(*id))))
                }
                _ => match ids.first() {
                    Some(&first) => self.call_node(first, method, args),
                    None => Ok(Val::Null),
                },
            },
            Obj::Children(parent) => self.call_list(*parent, method, args),
            Obj::Node(id) => self.call_node(*id, method, args),
            Obj::Event | Obj::Data | Obj::Template | Obj::Prop { .. } => Ok(Val::Null),
        }
    }

    /// `nodes` list methods of a data node (PDFium `CJX_List`).
    fn call_list(&mut self, parent: usize, method: &str, args: &[Val]) -> SResult<Val> {
        let node_arg = |index: usize| match args.get(index) {
            Some(Val::Obj(Obj::Node(id))) => Some(*id),
            Some(Val::Obj(Obj::Nodes(ids))) if ids.len() == 1 => Some(ids[0]),
            _ => None,
        };
        match method {
            "item" => {
                if args.len() != 1 {
                    return err(
                        "XFAObject.item: Incorrect number of parameters passed to function.",
                    );
                }
                let index = Self::arg_num(args, 0) as i64;
                Ok(usize::try_from(index)
                    .ok()
                    .and_then(|i| self.nodes[parent].children.get(i))
                    .map_or(Val::Null, |id| Val::Obj(Obj::Node(*id))))
            }
            "namedItem" => {
                let name = Self::arg_text(args, 0);
                Ok(self
                    .children_named(parent, &name)
                    .first()
                    .map_or(Val::Null, |id| Val::Obj(Obj::Node(*id))))
            }
            "append" => {
                if args.len() != 1 {
                    return err(
                        "XFAObject.append: Incorrect number of parameters passed to function.",
                    );
                }
                let Some(child) = node_arg(0) else {
                    return err("XFAObject.append: Invalid argument.");
                };
                self.attach_data(parent, child, None)?;
                Ok(Val::Null)
            }
            "insert" => {
                if args.len() != 2 {
                    return err(
                        "XFAObject.insert: Incorrect number of parameters passed to function.",
                    );
                }
                let (Some(child), Some(before)) = (node_arg(0), node_arg(1)) else {
                    return err("XFAObject.insert: Invalid argument.");
                };
                let Some(position) = self.nodes[parent]
                    .children
                    .iter()
                    .position(|&c| c == before)
                else {
                    return err("XFAObject.insert: Invalid argument.");
                };
                // Detaching `child` first may shift `before` left.
                let shift = usize::from(
                    self.nodes[child].parent == Some(parent)
                        && self.nodes[parent].children.iter().position(|&c| c == child)
                            < Some(position),
                );
                self.attach_data(parent, child, Some(position - shift))?;
                Ok(Val::Null)
            }
            "remove" => {
                if args.len() != 1 {
                    return err(
                        "XFAObject.remove: Incorrect number of parameters passed to function.",
                    );
                }
                let Some(child) = node_arg(0) else {
                    return err("XFAObject.remove: Invalid argument.");
                };
                if self.nodes[child].parent == Some(parent) && self.nodes[child].class.is_data() {
                    self.mutate()?;
                    self.nodes[parent].children.retain(|&c| c != child);
                    self.nodes[child].parent = None;
                }
                Ok(Val::Null)
            }
            _ => err(format!("unknown method {method}")),
        }
    }

    /// Data-node methods (PDFium `CJX_Node`, `CJX_Model`); `None` when
    /// `method` is not data-specific.
    fn call_data_node(&mut self, id: usize, method: &str, args: &[Val]) -> Option<SResult<Val>> {
        Some(match method {
            "isRecordGroup" => Ok(Val::Bool(self.record() == Some(id))),
            "getAttribute" => {
                if args.len() != 1 {
                    return Some(err(
                        "XFAObject.getAttribute: Incorrect number of parameters passed to function.",
                    ));
                }
                let name = Self::arg_text(args, 0);
                Ok(Val::Str(
                    self.meta_child(id, &name)
                        .and_then(|child| self.nodes[child].value.clone())
                        .unwrap_or_default(),
                ))
            }
            "setAttribute" => {
                if args.len() != 2 {
                    return Some(err(
                        "XFAObject.setAttribute: Incorrect number of parameters passed to function.",
                    ));
                }
                // PDFium's argument order: value, then attribute name.
                let value = Self::arg_text(args, 0);
                let name = Self::arg_text(args, 1);
                if name.is_empty() || !name.chars().all(is_name_char) {
                    return Some(Ok(Val::Null));
                }
                let child = match self.meta_child(id, &name) {
                    Some(child) => child,
                    None => {
                        let child = match self.new_data_node(Class::DataValue, &name) {
                            Ok(child) => child,
                            Err(error) => return Some(Err(error)),
                        };
                        self.nodes[child].meta = true;
                        if let Err(error) = self.attach_data(id, child, Some(0)) {
                            return Some(Err(error));
                        }
                        child
                    }
                };
                self.set_value(child, &Val::Str(value)).map(|_| Val::Null)
            }
            "createNode" => {
                if args.is_empty() || args.len() > 3 {
                    return Some(err(
                        "XFAObject.createNode: Incorrect number of parameters passed to function.",
                    ));
                }
                let class = match Self::arg_text(args, 0).as_str() {
                    "dataGroup" => Class::DataGroup,
                    "dataValue" => Class::DataValue,
                    _ => return Some(Ok(Val::Null)),
                };
                let name = Self::arg_text(args, 1);
                self.new_data_node(class, &name)
                    .map(|node| Val::Obj(Obj::Node(node)))
            }
            "saveXML" => {
                let mut out = String::new();
                self.data_xml(id, &mut out, 0);
                Ok(Val::Str(out))
            }
            "assignNode" => {
                // A no-op in PDFium as well (pdfium:501).
                if args.is_empty() || args.len() > 3 {
                    return Some(err(
                        "XFAObject.assignNode: Incorrect number of parameters passed to function.",
                    ));
                }
                Ok(Val::Null)
            }
            "clearErrorList" => Ok(Val::Null),
            _ => return None,
        })
    }

    /// `execValidate()`: queues the subtree's validation (scripts run
    /// after the current script) and returns whether its null and format
    /// tests pass (always true before the initial validation pass).
    fn exec_validate(&mut self, id: usize, args: &[Val]) -> SResult<Val> {
        if !args.is_empty() {
            return err(
                "XFAObject.execValidate: Incorrect number of parameters passed to function.",
            );
        }
        if !self.validations_enabled {
            return Ok(Val::Bool(true));
        }
        self.requests.push(Request::Validate(id));
        if !self.validate_ready {
            return Ok(Val::Bool(true));
        }
        let mut stack = vec![id];
        let mut seen = 0usize;
        while let Some(node) = stack.pop() {
            seen += 1;
            if seen > 100_000 {
                break;
            }
            if !self.static_validation(node).is_empty() {
                return Ok(Val::Bool(false));
            }
            stack.extend(self.nodes[node].children.iter().copied());
        }
        Ok(Val::Bool(true))
    }

    /// Whether `id` takes part in validation: a field, exclusion group or
    /// subform with a `<validate>`, not `inactive` (itself or an ancestor).
    pub(crate) fn validatable(&self, id: usize) -> bool {
        let node = &self.nodes[id];
        if node.validate.is_none()
            || !matches!(node.class, Class::Field | Class::ExclGroup | Class::Subform)
        {
            return false;
        }
        let mut current = Some(id);
        let mut guard = 0;
        while let Some(n) = current {
            guard += 1;
            if guard > 256 || self.nodes[n].presence == "inactive" {
                return false;
            }
            current = self.nodes[n].parent;
        }
        true
    }

    /// The null and format tests of `id` (PDFium
    /// `ProcessNullTestValidate`, `ProcessFormatTestValidate`).
    pub(crate) fn static_validation(&self, id: usize) -> Vec<ValidationFailure> {
        let mut out = Vec::new();
        if !self.validatable(id) {
            return out;
        }
        let node = &self.nodes[id];
        let Some(validate) = node.validate.as_deref() else {
            return out;
        };
        if !matches!(node.class, Class::Field | Class::ExclGroup) {
            return out;
        }
        let value = node.value.clone().unwrap_or_default();
        if value.is_empty() {
            if validate.null_test != "disabled" {
                let message = validate.null_message.clone().unwrap_or_else(|| {
                    let name = self.validate_caption_name(id);
                    if validate.null_test == "error" {
                        format!("{name} cannot be blank.")
                    } else {
                        format!(
                            "{name} cannot be blank. To ignore validations for {name}, click Ignore."
                        )
                    }
                });
                out.push(ValidationFailure {
                    test: "nullTest",
                    severity: validate.null_test.clone(),
                    message,
                });
            }
            return out;
        }
        if let Some(picture) = &validate.picture {
            if validate.format_test != "disabled"
                && !picture::validate_value_in(
                    &value,
                    picture,
                    &node.value_kind,
                    &self.locales,
                    node.locale.as_deref(),
                )
            {
                let message = validate
                    .format_message
                    .clone()
                    .unwrap_or_else(|| self.validate_message(id, &validate.format_test));
                out.push(ValidationFailure {
                    test: "formatTest",
                    severity: validate.format_test.clone(),
                    message,
                });
            }
        }
        out
    }

    /// The caption text, else the name (PDFium `GetValidateCaptionName`).
    fn validate_caption_name(&self, id: usize) -> String {
        let node = &self.nodes[id];
        node.caption
            .clone()
            .map(|caption| caption.trim().to_string())
            .filter(|caption| !caption.is_empty())
            .unwrap_or_else(|| node.name.clone())
    }

    /// PDFium `GetValidateMessage` (XFA 2.8+ wording).
    pub(crate) fn validate_message(&self, id: usize, severity: &str) -> String {
        let name = self.validate_caption_name(id);
        if severity == "error" {
            format!("The value you entered for {name} is invalid.")
        } else {
            format!(
                "The value you entered for {name} is invalid. To ignore validations for {name}, click Ignore."
            )
        }
    }

    fn resolve_from_root(&mut self, som: &str) -> SResult<Vec<Obj>> {
        let segs = parse_som(som)?;
        let Some((first, rest)) = segs.split_first() else {
            return Ok(Vec::new());
        };
        // `xfa.resolveNode("form1.x")`: an unqualified first name is looked
        // up as a child of xfa, then of the form, then anywhere in it.
        let mut current = match &first.name {
            SomName::Name(name) if name == "datasets" => {
                self.data_objs("datasets").unwrap_or_default()
            }
            SomName::Name(name)
                if !matches!(
                    name.as_str(),
                    "xfa" | "form" | "host" | "layout" | "event" | "template" | "datasets"
                ) =>
            {
                let root = self.root;
                let mut group = if self.nodes[root].name == *name {
                    vec![root]
                } else {
                    self.children_named(root, name)
                };
                if group.is_empty() {
                    self.descendants_named(root, name, &mut group);
                }
                self.select(group, first.index, None)
                    .into_iter()
                    .map(Obj::Node)
                    .collect()
            }
            _ => self.start(None, first)?,
        };
        for seg in rest {
            if current.is_empty() {
                break;
            }
            current = self.step(&current, seg, None)?;
        }
        Ok(current)
    }

    fn call_node(&mut self, id: usize, method: &str, args: &[Val]) -> SResult<Val> {
        if self.nodes[id].class.is_data() {
            if let Some(result) = self.call_data_node(id, method, args) {
                return result;
            }
        }
        match method {
            "resolveNode" | "resolveNodes" => {
                let som = Self::arg_text(args, 0);
                let mut found = self.resolve(Some(id), &som)?;
                Ok(list_or_single(method == "resolveNodes", &mut found))
            }
            "execCalculate" => {
                self.requests
                    .push(Request::Event(id, Activity::Calculate, "calculate".into()));
                Ok(Val::Null)
            }
            "execInitialize" => {
                self.requests.push(Request::Event(
                    id,
                    Activity::Initialize,
                    "initialize".into(),
                ));
                Ok(Val::Null)
            }
            "execEvent" => {
                let activity = Self::arg_text(args, 0);
                if activity == "validate" {
                    // PDFium: a field reports the validation outcome.
                    let result = self.exec_validate(id, &[])?;
                    return Ok(if self.nodes[id].class == Class::Field {
                        result
                    } else {
                        Val::Null
                    });
                }
                let kind = activity_kind(&activity, None);
                self.requests.push(Request::Event(id, kind, activity));
                Ok(Val::Null)
            }
            "execValidate" => self.exec_validate(id, args),
            "addItem" => {
                let display = Self::arg_text(args, 0);
                let save = if args.len() > 1 {
                    Self::arg_text(args, 1)
                } else {
                    display.clone()
                };
                self.mutate()?;
                if self.nodes[id].items.len() >= 10_000 {
                    return err("too many items");
                }
                self.nodes[id].items.push((display, save));
                self.nodes[id].changes.items = true;
                Ok(Val::Null)
            }
            "clearItems" => {
                self.mutate()?;
                self.nodes[id].items.clear();
                self.nodes[id].changes.items = true;
                Ok(Val::Null)
            }
            "deleteItem" => {
                let index = Self::arg_num(args, 0) as i64;
                if let Ok(index) = usize::try_from(index) {
                    if index < self.nodes[id].items.len() {
                        self.mutate()?;
                        self.nodes[id].items.remove(index);
                        self.nodes[id].changes.items = true;
                        return Ok(Val::Bool(true));
                    }
                }
                Ok(Val::Bool(false))
            }
            "setItems" => {
                let text = Self::arg_text(args, 0);
                let per = (Self::arg_num(args, 1) as usize).clamp(1, 2);
                let parts: Vec<&str> = text.split(',').collect();
                let mut items = Vec::new();
                for chunk in parts.chunks(per) {
                    let display = chunk[0].to_string();
                    let save = chunk.get(1).map_or(display.clone(), |s| s.to_string());
                    items.push((display, save));
                }
                self.mutate()?;
                self.nodes[id].items = items;
                self.nodes[id].changes.items = true;
                Ok(Val::Bool(true))
            }
            "getDisplayItem" | "getSaveItem" => {
                let index = Self::arg_num(args, 0) as i64;
                Ok(usize::try_from(index)
                    .ok()
                    .and_then(|i| self.nodes[id].items.get(i))
                    .map_or(Val::Null, |(display, save)| {
                        Val::Str(if method == "getDisplayItem" {
                            display.clone()
                        } else {
                            save.clone()
                        })
                    }))
            }
            "boundItem" => {
                let display = Self::arg_text(args, 0);
                Ok(self.nodes[id]
                    .items
                    .iter()
                    .find(|(d, _)| *d == display)
                    .map_or(Val::Str(display.clone()), |(_, save)| {
                        Val::Str(save.clone())
                    }))
            }
            "getItemState" => {
                let index = Self::arg_num(args, 0) as i64;
                let value = self.nodes[id].value.clone().unwrap_or_default();
                Ok(Val::Bool(
                    usize::try_from(index)
                        .ok()
                        .and_then(|i| self.nodes[id].items.get(i))
                        .is_some_and(|(_, save)| *save == value),
                ))
            }
            "setItemState" => {
                let index = Self::arg_num(args, 0) as i64;
                let on = args.get(1).is_some_and(|v| v.to_num() != 0.0);
                if let Some(save) = usize::try_from(index)
                    .ok()
                    .and_then(|i| self.nodes[id].items.get(i))
                    .map(|(_, save)| save.clone())
                {
                    if on {
                        self.set_value(id, &Val::Str(save))?;
                    }
                }
                Ok(Val::Null)
            }
            "isPropertySpecified" => Ok(Val::Bool(false)),
            "getElement" | "setElement" | "applyXSL" | "assignNode" | "clone" | "getAttribute"
            | "setAttribute" | "saveXML" | "loadXML" | "relayout" => Ok(Val::Null),
            "isRecordGroup" => Ok(Val::Bool(false)),
            _ => err(format!("unknown method {method}")),
        }
    }

    fn call_layout(&mut self, method: &str, args: &[Val], this: Option<usize>) -> SResult<Val> {
        let node = match args.first() {
            Some(Val::Obj(Obj::Node(id))) => Some(*id),
            Some(Val::Obj(Obj::Nodes(ids))) => ids.first().copied(),
            _ => this,
        };
        let layout = self.layout.clone().unwrap_or_default();
        let count = layout.page_count.max(1);
        let page_of = |node: Option<usize>| -> usize {
            if let Some(page) = self.current_page {
                return page;
            }
            let mut current = node;
            while let Some(id) = current {
                if let Some(&page) = layout.node_pages.get(&id) {
                    return page;
                }
                current = self.nodes[id].parent;
            }
            0
        };
        Ok(match method {
            "pageCount"
            | "absPageCount"
            | "sheetCount"
            | "absPageCountInBatch"
            | "sheetCountInBatch" => Val::Num(count as f64),
            "page" | "sheet" | "absPageInBatch" | "sheetInBatch" => {
                Val::Num((page_of(node) + 1) as f64)
            }
            "absPage" => Val::Num(page_of(node) as f64),
            "pageSpan" => Val::Num(1.0),
            "h" | "w" | "x" | "y" => {
                let rect = node
                    .and_then(|id| layout.node_rects.get(&id).copied())
                    .unwrap_or_default();
                let points = match method {
                    "x" => rect[0],
                    "y" => rect[1],
                    "w" => rect[2],
                    _ => rect[3],
                };
                let unit = match args.get(1) {
                    Some(value) => value.to_text(),
                    None => "pt".to_string(),
                };
                Val::Num(match unit.as_str() {
                    "in" => points / 72.0,
                    "cm" => points / 72.0 * 2.54,
                    "mm" => points / 72.0 * 25.4,
                    "mp" => points * 1000.0,
                    _ => points,
                })
            }
            "relayout" | "relayoutPageArea" => Val::Null,
            "pageContent" => Val::Obj(Obj::Nodes(Vec::new())),
            _ => return err(format!("unknown method {method}")),
        })
    }
}

/// Property elements reachable as `#name` SOM segments.
const PROPERTY_CLASSES: &[&str] = &[
    "caption",
    "value",
    "font",
    "border",
    "margin",
    "para",
    "items",
    "ui",
    "format",
    "bind",
    "occur",
    "calculate",
    "validate",
    "assist",
    "extras",
    "desc",
    "fill",
];

fn typed(text: Option<&str>, numeric: bool) -> Val {
    match text {
        None | Some("") => Val::Null,
        Some(text) if numeric => match text.trim().parse::<f64>() {
            Ok(value) if value.is_finite() => Val::Num(value),
            _ => Val::Str(text.to_string()),
        },
        Some(text) => Val::Str(text.to_string()),
    }
}

fn list_or_single(list: bool, found: &mut Vec<Obj>) -> Val {
    let mut ids = Vec::new();
    let mut others = Vec::new();
    for obj in found.drain(..) {
        match obj {
            Obj::Node(id) => ids.push(id),
            Obj::Nodes(more) => ids.extend(more),
            other => others.push(other),
        }
    }
    if list {
        return Val::Obj(Obj::Nodes(ids));
    }
    if let Some(&first) = ids.first() {
        return Val::Obj(Obj::Node(first));
    }
    others.into_iter().next().map_or(Val::Null, Val::Obj)
}

/// Maps an `<event activity>` (+ `ref`) to the renderer's activity.
pub(crate) fn activity_kind(activity: &str, reference: Option<&str>) -> Activity {
    match (activity, reference.map(str::trim)) {
        ("initialize", _) => Activity::Initialize,
        ("calculate", _) => Activity::Calculate,
        ("docReady", _) => Activity::DocReady,
        ("ready", Some("$layout")) => Activity::LayoutReady,
        ("ready", _) => Activity::FormReady,
        ("layoutReady", _) => Activity::LayoutReady,
        ("formReady", _) => Activity::FormReady,
        _ => Activity::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// form1 { #subform { a[0], a[1] { x } }, total, row[0..2] { amt } }
    pub(crate) fn sample() -> Dom {
        let mut dom = Dom::new(Limits::default(), LocaleSet::default());
        let root = dom.add(SNode::new(Class::Subform, "form1", 0), None);
        dom.root = root;
        let page = dom.add(SNode::new(Class::Subform, "", 1), Some(root));
        dom.add(SNode::new(Class::Field, "a", 2), Some(page));
        let a1 = dom.add(SNode::new(Class::Subform, "a", 2), Some(page));
        let mut x = SNode::new(Class::Field, "x", 3);
        x.value = Some("hello".into());
        dom.add(x, Some(a1));
        let mut total = SNode::new(Class::Field, "total", 4);
        total.value_kind = "decimal".into();
        dom.add(total, Some(root));
        for value in ["1", "2.5", "4"] {
            let mut row = SNode::new(Class::Subform, "row", 5);
            row.occur = Some((0, None));
            let row = dom.add(row, Some(root));
            let mut amt = SNode::new(Class::Field, "amt", 6);
            amt.value_kind = "decimal".into();
            amt.value = Some(value.into());
            dom.add(amt, Some(row));
        }
        dom
    }

    #[test]
    fn som_paths_resolve_through_transparent_subforms() {
        let mut dom = sample();
        let x = dom.resolve(None, "xfa.form.form1.a[1].x").unwrap();
        assert_eq!(x.len(), 1);
        assert_eq!(dom.value_of(&x[0]).unwrap(), Val::Str("hello".into()));
        let rows = dom.resolve(None, "$form.form1.row[*].amt").unwrap();
        assert_eq!(rows.len(), 3);
        let all = dom.resolve(None, "form1..amt").unwrap();
        assert_eq!(all.len(), 1);
        let all = dom.resolve(None, "form1..amt[*]").unwrap();
        assert_eq!(all.len(), 3);
        let class = dom.resolve(None, "form1.#subform[0].a[0]").unwrap();
        assert_eq!(class.len(), 1);
        // Scoping: from row[2].amt, `amt` is its own, `row[-1]` the previous row.
        let Obj::Node(amt2) = dom.resolve(None, "form1.row[2].amt").unwrap()[0].clone() else {
            panic!()
        };
        let own = dom.resolve(Some(amt2), "amt").unwrap();
        assert_eq!(own, vec![Obj::Node(amt2)]);
        let previous = dom.resolve(Some(amt2), "row[-1].amt").unwrap();
        assert_eq!(dom.value_of(&previous[0]).unwrap(), Val::Num(2.5));
        let manager = dom.resolve(Some(amt2), "_row").unwrap();
        assert!(matches!(manager[0], Obj::Manager { .. }));
        assert_eq!(dom.get(&manager[0], "count").unwrap(), Val::Num(3.0));
        // `form1._row` reaches the same manager through its parent.
        let root = dom.root;
        assert_eq!(
            dom.get(&Obj::Node(root), "_row").unwrap(),
            Val::Obj(manager[0].clone())
        );
    }

    #[test]
    fn som_parse_forms() {
        let segs = parse_som("$form..x[*].#field[+1]").unwrap();
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0].name, SomName::Dollar("form".into()));
        assert!(segs[1].desc);
        assert_eq!(segs[1].index, SomIndex::All);
        assert_eq!(segs[2].index, SomIndex::Rel(1));
        let segs = parse_som("!data.x").unwrap();
        assert_eq!(segs[0].name, SomName::Bang);
        assert!(parse_som("a[").is_err());
    }

    #[test]
    fn number_text() {
        assert_eq!(format_number(3.0), "3");
        assert_eq!(format_number(0.1 + 0.2), "0.30000000000000004");
        assert_eq!(format_number(-2.5), "-2.5");
        assert_eq!(parse_leading_number("12abc"), 12.0);
        assert_eq!(parse_leading_number("abc"), 0.0);
        assert_eq!(parse_leading_number(" 1e3 "), 1000.0);
    }
}
