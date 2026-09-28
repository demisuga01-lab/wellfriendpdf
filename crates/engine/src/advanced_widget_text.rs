//! Coordinated text-field values and native widget appearance edits. Every
//! widget is selected from the authoritative /Fields-/Kids ownership graph;
//! no name match, page overlay, implicit JavaScript, or partial publication.
use super::*;
use crate::annotation_promotion::{reachable_field_nodes, ReachableFieldNode};
use serde_json::{json, Value};

const MAX_WIDGETS: usize = 256;
const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WidgetTextTarget {
    pub input_sha256: String,
    /// Review page; must contain a widget of the selected terminal field.
    pub page: usize,
    pub field: ObjectRef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WidgetAppearanceEdit {
    pub appearance: AppearanceTextEditRequest,
    /// Whole selected normal appearance, including descendants. Formatting
    /// need not equal the field value, but both mappings are explicit.
    pub expected_display: String,
    pub replacement_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WidgetDefaultAppearance {
    /// Preserve DA/DR/Q and widget overrides byte-for-byte. Existing defaults
    /// remain the viewer's future editing style, not a recovered original font.
    PreserveSourceDefaults,
    /// Reuse a font from one saved source scope and make it available through
    /// AcroForm DR. All widget DA overrides are removed under this decision.
    FromEditedAppearance {
        widget: ObjectRef,
        font_resource: String,
        font_size: f64,
        rgb: [f64; 3],
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WidgetTextEditRequest {
    pub target: WidgetTextTarget,
    pub expected_value: String,
    pub replacement_value: String,
    pub widgets: Vec<WidgetAppearanceEdit>,
    pub default_appearance: WidgetDefaultAppearance,
    #[serde(default)]
    pub update_default_value: bool,
    #[serde(default)]
    pub discard_rich_text: bool,
    #[serde(default)]
    pub allow_read_only: bool,
    /// Explicitly preserve actions/calculation order without executing them.
    /// A viewer may subsequently execute those retained actions.
    #[serde(default)]
    pub preserve_actions_without_execution: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct WidgetFieldSource {
    pub target: WidgetTextTarget,
    pub value: String,
    pub flags: i64,
    pub widgets: Vec<Value>,
    pub default_appearance: Value,
    pub limits: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WidgetTextEditReport {
    pub schema_version: String,
    pub target_before: WidgetTextTarget,
    pub target_after: WidgetTextTarget,
    pub value_before: String,
    pub value_after: String,
    pub widgets: Vec<AppearanceTextEditReport>,
    pub all_widget_displays_verified: bool,
    pub field_ownership_verified: bool,
    pub single_published_revision: bool,
    pub native_reports_scope: String,
    pub default_appearance: Value,
    pub default_value_updated: bool,
    pub rich_text_discarded: bool,
    pub actions_preserved_without_execution: bool,
    /// The physical widgets checked by this transaction, not necessarily the
    /// complete semantic invalidation set when shared structure owners change.
    pub widget_pages: Vec<usize>,
    pub affected_pages: Vec<usize>,
    pub affected_pages_scope: String,
    pub limits: Vec<String>,
}

fn unsupported(message: &str) -> WellfriendError {
    WellfriendError::UnsupportedFeature(format!("coordinated widget text: {message}"))
}
fn resolved(reader: &PdfReader, dict: &PdfDictionary, key: &str) -> Result<Option<PdfObject>> {
    dict.get(key).map(|v| reader.resolve(v.clone())).transpose()
}
fn dict(reader: &PdfReader, value: &PdfObject) -> Result<PdfDictionary> {
    reader
        .resolve(value.clone())?
        .as_dict()
        .cloned()
        .ok_or_else(|| fail("invalid field dictionary"))
}
fn inherited(
    reader: &PdfReader,
    chain: &[ReachableFieldNode],
    key: &str,
) -> Result<Option<PdfObject>> {
    for node in chain.iter().rev() {
        if let Some(value) = resolved(reader, &node.dictionary, key)? {
            return Ok(Some(value));
        }
    }
    Ok(None)
}
fn text(value: Option<PdfObject>) -> Result<String> {
    match value {
        None | Some(PdfObject::Null) => Ok(String::new()),
        Some(PdfObject::String(bytes)) if bytes.len() <= MAX_TEXT_BYTES => {
            Ok(crate::info::decode_pdf_text_string(&bytes))
        }
        _ => Err(fail("field value is not a bounded text string")),
    }
}
fn integer(value: Option<PdfObject>, default: i64) -> Result<i64> {
    match value {
        None | Some(PdfObject::Null) => Ok(default),
        Some(PdfObject::Integer(value)) => Ok(value),
        _ => Err(fail("invalid field integer")),
    }
}

struct Binding {
    chain: Vec<ReachableFieldNode>,
    field: PdfDictionary,
    form: PdfDictionary,
    widgets: BTreeMap<ObjectRef, (usize, usize)>,
    value: String,
    flags: i64,
}

// Revision-local indexes are shared by all discovered fields. Do not rebuild
// the full field/page/annotation graph once for every field in a page window.
struct FieldContext<'a> {
    engine: &'a ContentEngine,
    nodes: Vec<ReachableFieldNode>,
    paths: BTreeMap<Vec<usize>, usize>,
    references: BTreeMap<ObjectRef, usize>,
    children: BTreeMap<Vec<usize>, Vec<usize>>,
    locations: BTreeMap<ObjectRef, (usize, usize)>,
    claimed_widgets: BTreeMap<ObjectRef, BTreeSet<ObjectRef>>,
    pages: BTreeMap<usize, ObjectRef>,
    form: PdfDictionary,
}

impl<'a> FieldContext<'a> {
    fn read(engine: &'a ContentEngine) -> Result<Self> {
        let document = engine.document();
        let reader = document.reader();
        let nodes = reachable_field_nodes(document)?;
        let mut paths = BTreeMap::new();
        let mut references = BTreeMap::new();
        let mut children = BTreeMap::<Vec<usize>, Vec<usize>>::new();
        for (index, node) in nodes.iter().enumerate() {
            crate::cancel::check_current_cancel("widget field index")?;
            paths.insert(node.path.clone(), index);
            if let Some(id) = node.reference {
                references.insert(id, index);
            }
            if let Some((_, parent)) = node.path.split_last() {
                children.entry(parent.to_vec()).or_default().push(index);
            }
        }
        let identities = crate::annotation_identity::index(document, 100_000)?;
        let mut locations = BTreeMap::new();
        let mut claimed_widgets = BTreeMap::<ObjectRef, BTreeSet<ObjectRef>>::new();
        for ((page, index), identity) in identities {
            crate::cancel::check_current_cancel("widget page ownership index")?;
            let Some(id) = identity.reference else {
                continue;
            };
            locations.insert(id, (page, index));
            let value = annotation(reader, id)?;
            if value.get_name("Subtype") == Some("Widget") {
                if let Some(parent) = value.get_reference("Parent") {
                    claimed_widgets.entry(parent).or_default().insert(id);
                }
            }
        }
        let pages = document
            .get_pages()?
            .into_iter()
            .map(|page| {
                (
                    page.page_number,
                    (page.object_number, page.generation_number),
                )
            })
            .collect();
        let catalog = document.get_catalog()?;
        let form = match resolved(reader, &catalog, "AcroForm")? {
            None | Some(PdfObject::Null) => PdfDictionary::empty(),
            Some(PdfObject::Dictionary(form)) => form,
            _ => return Err(fail("invalid AcroForm dictionary")),
        };
        Ok(Self {
            engine,
            nodes,
            paths,
            references,
            children,
            locations,
            claimed_widgets,
            pages,
            form,
        })
    }

    fn bind(&self, target: &WidgetTextTarget) -> Result<Binding> {
        let reader = self.engine.document().reader();
        let index = self
            .references
            .get(&target.field)
            .ok_or_else(|| fail("field is not in the reachable field ownership graph"))?;
        let node = &self.nodes[*index];
        let chain = (1..=node.path.len())
            .map(|length| {
                self.paths
                    .get(&node.path[..length])
                    .map(|index| self.nodes[*index].clone())
                    .ok_or_else(|| fail("reachable field ancestor disappeared"))
            })
            .collect::<Result<Vec<_>>>()?;
        let field_type = inherited(reader, &chain, "FT")?;
        if field_type.as_ref().and_then(PdfObject::as_name) != Some("Tx") {
            return Err(unsupported(
                "this transaction edits text fields, not choice, button or signature values",
            ));
        }
        let mut widget_ids = BTreeSet::new();
        if node.widget {
            widget_ids.insert(target.field);
        }
        for index in self.children.get(&node.path).into_iter().flatten() {
            let child = &self.nodes[*index];
            if !child.widget
                || child.dictionary.contains_key("T")
                || child.dictionary.contains_key("FT")
            {
                return Err(fail(
                    "selected field is not a terminal owner of pure widget children",
                ));
            }
            widget_ids.insert(child.reference.ok_or_else(|| {
                unsupported("materialize direct widget owners before editing the field")
            })?);
        }
        if widget_ids.is_empty() || widget_ids.len() > MAX_WIDGETS {
            return Err(unsupported("expected 1..=256 uniquely owned widgets"));
        }
        if self
            .claimed_widgets
            .get(&target.field)
            .is_some_and(|claimed| !claimed.is_subset(&widget_ids))
        {
            return Err(fail(
                "page widget claims this field but is absent from its Kids",
            ));
        }
        let mut widgets = BTreeMap::new();
        for id in widget_ids {
            crate::cancel::check_current_cancel("widget page ownership")?;
            let &(page, index) = self
                .locations
                .get(&id)
                .ok_or_else(|| fail("field widget is absent from page annotations"))?;
            let annotation = annotation(reader, id)?;
            if annotation.get("P").is_some_and(|v| !v.is_null())
                && annotation.get_reference("P") != self.pages.get(&page).copied()
            {
                return Err(fail("widget P disagrees with its page occurrence"));
            }
            widgets.insert(id, (page, index));
        }
        if !widgets.values().any(|(page, _)| *page == target.page) {
            return Err(fail(
                "every widget must have one page occurrence, including the review page",
            ));
        }
        let flags = integer(inherited(reader, &chain, "Ff")?, 0)?;
        if flags < 0 || flags > u32::MAX as i64 {
            return Err(fail("invalid field flags"));
        }
        let value = text(inherited(reader, &chain, "V")?)?;
        Ok(Binding {
            chain,
            field: node.dictionary.clone(),
            form: self.form.clone(),
            widgets,
            value,
            flags,
        })
    }
}

impl Binding {
    fn read(engine: &ContentEngine, target: &WidgetTextTarget) -> Result<Self> {
        FieldContext::read(engine)?.bind(target)
    }

    fn validate(&self, engine: &ContentEngine, request: &WidgetTextEditRequest) -> Result<bool> {
        let reader = engine.document().reader();
        if self.value != request.expected_value {
            return Err(fail("field value compare-and-swap failed"));
        }
        if self.form.get("XFA").is_some_and(|v| !v.is_null()) {
            return Err(unsupported(
                "hybrid XFA values require a coordinated datasets transaction",
            ));
        }
        match resolved(reader,&self.form,"NeedAppearances")? {
            None|Some(PdfObject::Null)|Some(PdfObject::Boolean(false))=>{},
            Some(PdfObject::Boolean(true))=>return Err(unsupported("NeedAppearances requests document-wide regeneration; materialize all field appearances first")),
            _=>return Err(fail("invalid NeedAppearances flag")),
        }
        if self.flags & 1 != 0 && !request.allow_read_only {
            return Err(unsupported(
                "read-only field modification requires an explicit decision",
            ));
        }
        // Password displays, file selectors and comb cells have specialized
        // semantics not established by this ordinary text/source transaction.
        if self.flags & ((1 << 13) | (1 << 20) | (1 << 24)) != 0 {
            return Err(unsupported("password, file-selection and comb fields need their specialized display transaction"));
        }
        if let Some(value) = inherited(reader, &self.chain, "MaxLen")?.filter(|v| !v.is_null()) {
            let maximum = integer(Some(value), 0)?;
            if maximum < 0 || request.replacement_value.chars().count() as u64 > maximum as u64 {
                return Err(fail("replacement exceeds the field's MaxLen"));
            }
        }
        if self.flags & (1 << 12) == 0
            && request
                .replacement_value
                .chars()
                .any(|c| c == '\r' || c == '\n')
        {
            return Err(fail(
                "single-line text field cannot receive paragraph breaks",
            ));
        }
        let mut actions = self
            .form
            .get("CO")
            .map(|v| reader.resolve(v.clone()))
            .transpose()?
            .is_some_and(|v| match v {
                PdfObject::Null => false,
                PdfObject::Array(items) => !items.is_empty(),
                _ => true,
            });
        let mut rich = self.flags & (1 << 25) != 0;
        for node in &self.chain {
            actions |= ["AA", "A"]
                .iter()
                .any(|key| node.dictionary.get(key).is_some_and(|v| !v.is_null()));
        }
        for id in self.widgets.keys() {
            let annotation = annotation(reader, *id)?;
            actions |= ["AA", "A"]
                .iter()
                .any(|key| annotation.get(key).is_some_and(|v| !v.is_null()));
            rich |= ["RV", "DS"]
                .iter()
                .any(|key| annotation.get(key).is_some_and(|v| !v.is_null()));
            let ap = dict(
                reader,
                annotation
                    .get("AP")
                    .ok_or_else(|| unsupported("every widget needs a source appearance"))?,
            )?;
            if ["R", "D"]
                .iter()
                .any(|key| ap.get(key).is_some_and(|v| !v.is_null()))
                || !matches!(resolved(reader, &ap, "N")?, Some(PdfObject::Stream { .. }))
            {
                return Err(unsupported("alternate or stateful widget appearances require coordinated state regeneration"));
            }
        }
        rich |= ["RV", "DS"]
            .iter()
            .any(|key| self.field.get(key).is_some_and(|v| !v.is_null()));
        if rich && !request.discard_rich_text {
            return Err(unsupported(
                "rich text requires explicit plain-text conversion or a rich-text transaction",
            ));
        }
        if actions && !request.preserve_actions_without_execution {
            return Err(unsupported(
                "retained form actions/calculations require explicit no-execution approval",
            ));
        }
        Ok(actions)
    }
}

// One scoped extraction per source page/revision, rather than one full page
// traversal per widget. Never carry this cache across an appearance mutation.
struct PageDisplays<'a> {
    engine: &'a ContentEngine,
    pages: BTreeMap<usize, BTreeMap<ObjectRef, String>>,
    bytes: usize,
}
impl<'a> PageDisplays<'a> {
    fn new(engine: &'a ContentEngine) -> Self {
        Self {
            engine,
            pages: BTreeMap::new(),
            bytes: 0,
        }
    }
    fn text(&mut self, page: usize, widget: ObjectRef) -> Result<&str> {
        if !self.pages.contains_key(&page) {
            let chunks = self
                .engine
                .collect_page_scoped_text_chunks_including_appearances(
                    page,
                    &crate::text::TextTraversalLimits::default(),
                    &crate::cancel::current_cancel_token(),
                )?;
            let mut values = BTreeMap::<ObjectRef, String>::new();
            for chunk in chunks {
                let Some(owner) = chunk.appearance else {
                    continue;
                };
                let text = values.entry(owner.annotation).or_default();
                self.bytes = self.bytes.saturating_add(chunk.chunk.text.len());
                if self.bytes > 16 * 1024 * 1024
                    || text.len().saturating_add(chunk.chunk.text.len()) > MAX_TEXT_BYTES
                {
                    return Err(unsupported("widget display text budget exceeded"));
                }
                text.push_str(&chunk.chunk.text);
            }
            self.pages.insert(page, values);
        }
        Ok(self
            .pages
            .get(&page)
            .and_then(|values| values.get(&widget))
            .map(String::as_str)
            .unwrap_or(""))
    }
}

pub fn discover_widget_fields(input: &[u8], pages: &[usize]) -> Result<Vec<WidgetFieldSource>> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let context = FieldContext::read(&engine)?;
    let page_set = pages.iter().copied().collect::<BTreeSet<_>>();
    let page_count = engine.page_count()?;
    if page_set.len() != pages.len() || pages.iter().any(|page| *page == 0 || *page > page_count) {
        return Err(fail("widget discovery pages must be distinct and in range"));
    }
    let mut displays = PageDisplays::new(&engine);
    let revision = format!("{:x}", Sha256::digest(input));
    let mut selected = BTreeMap::new();
    for node in &context.nodes {
        if !node.widget {
            continue;
        }
        let Some(id) = node.reference else { continue };
        let page = context
            .locations
            .get(&id)
            .map(|(page, _)| *page)
            .filter(|page| page_set.contains(page));
        let Some(page) = page else { continue };
        let owner = if node.dictionary.contains_key("T")
            || node.dictionary.contains_key("FT")
            || node.path.len() == 1
        {
            id
        } else {
            node.dictionary.get_reference("Parent").ok_or_else(|| {
                unsupported("materialize direct field ancestors before widget discovery")
            })?
        };
        selected.entry(owner).or_insert(page);
    }
    if selected.len() > MAX_WIDGETS {
        return Err(unsupported(
            "widget discovery field budget exceeded; request fewer pages",
        ));
    }
    let mut result = Vec::new();
    let mut text_budget = 0usize;
    for (field, page) in selected {
        crate::cancel::check_current_cancel("widget field discovery")?;
        let target = WidgetTextTarget {
            input_sha256: revision.clone(),
            page,
            field,
        };
        let binding = match context.bind(&target) {
            Ok(value) => value,
            Err(WellfriendError::UnsupportedFeature(_)) => continue,
            Err(e) => return Err(e),
        };
        let mut widgets = Vec::new();
        for (id, (page, index)) in &binding.widgets {
            let display = displays.text(*page, *id)?;
            text_budget = text_budget.saturating_add(display.len());
            if text_budget > 16 * 1024 * 1024 {
                return Err(unsupported(
                    "widget discovery aggregate text budget exceeded",
                ));
            }
            widgets.push(json!({"annotation":id,"page":page,"annotation_index":index,"display_text":display}));
        }
        let da = inherited(engine.document().reader(), &binding.chain, "DA")?.or(resolved(
            engine.document().reader(),
            &binding.form,
            "DA",
        )?);
        let da = text(da)?;
        text_budget = text_budget
            .saturating_add(binding.value.len())
            .saturating_add(da.len());
        if text_budget > 16 * 1024 * 1024 {
            return Err(unsupported(
                "widget discovery aggregate text budget exceeded",
            ));
        }
        result.push(WidgetFieldSource{target,value:binding.value,flags:binding.flags,widgets,
            default_appearance:json!({"source_program":da,"source_defaults_preservable":true}),
            limits:vec!["all widgets and the terminal field value form one approved transaction".into(),"discovery does not establish editability; stateful, rich, scripted, XFA and specialized field semantics require explicit policies or further transactions".into()]});
    }
    Ok(result)
}

pub fn edit_widget_text(
    input: &[u8],
    request: &WidgetTextEditRequest,
    font_bytes: Option<&[u8]>,
) -> Result<(Vec<u8>, WidgetTextEditReport)> {
    crate::cancel::check_current_cancel("widget transaction input")?;
    let revision = format!("{:x}", Sha256::digest(input));
    if revision != request.target.input_sha256 {
        return Err(fail("stale field revision"));
    }
    let request_text_bytes = request.widgets.iter().fold(
        request
            .expected_value
            .len()
            .saturating_add(request.replacement_value.len()),
        |sum, widget| {
            sum.saturating_add(widget.expected_display.len())
                .saturating_add(widget.replacement_display.len())
                .saturating_add(widget.appearance.edit.replacement_text.len())
                .saturating_add(
                    widget
                        .appearance
                        .tagged_clone
                        .actual_text_updates
                        .iter()
                        .fold(0usize, |total, update| {
                            total
                                .saturating_add(update.expected_text.len())
                                .saturating_add(update.replacement_text.len())
                        }),
                )
        },
    );
    if request.widgets.is_empty()
        || request.widgets.len() > MAX_WIDGETS
        || request.expected_value.len() > MAX_TEXT_BYTES
        || request
            .replacement_value
            .encode_utf16()
            .count()
            .saturating_mul(2)
            .saturating_add(2)
            > MAX_TEXT_BYTES
        || request_text_bytes > 16 * 1024 * 1024
    {
        return Err(unsupported("field/widget request budget exceeded"));
    }
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let source = Binding::read(&engine, &request.target)?;
    let mut original_displays = PageDisplays::new(&engine);
    let actions = source.validate(&engine, request)?;
    let requested = request
        .widgets
        .iter()
        .map(|w| w.appearance.target.annotation)
        .collect::<BTreeSet<_>>();
    if requested.len() != request.widgets.len()
        || requested != source.widgets.keys().copied().collect()
    {
        return Err(fail("request must cover each field widget exactly once"));
    }
    let mut logical_updates = BTreeMap::<ObjectRef, (String, String)>::new();
    for widget in &request.widgets {
        let mut local = BTreeSet::new();
        for update in &widget.appearance.tagged_clone.actual_text_updates {
            if !local.insert(update.element) {
                return Err(fail("duplicate per-widget ActualText decision"));
            }
            let pair = (
                update.expected_text.clone(),
                update.replacement_text.clone(),
            );
            if let Some(previous) = logical_updates.insert(update.element, pair.clone()) {
                if previous != pair {
                    return Err(fail(
                        "widgets contain competing logical-text decisions for the same owner",
                    ));
                }
            }
            if logical_updates.len() > 1024 {
                return Err(unsupported("field logical-owner budget exceeded"));
            }
            let owner = annotation(engine.document().reader(), update.element)?;
            if text(resolved(engine.document().reader(), &owner, "ActualText")?)?
                != update.expected_text
            {
                return Err(fail("field logical owner compare-and-swap failed"));
            }
        }
    }
    // Bind every original source occurrence and complete display before any
    // mutation. The individual native writer still checks selected operands.
    let mut page_targets = BTreeMap::<usize, BTreeSet<Vec<u8>>>::new();
    let mut target_bytes = 0usize;
    for widget in &request.widgets {
        let target = &widget.appearance.target;
        if target.input_sha256 != revision
            || source.widgets.get(&target.annotation)
                != Some(&(target.page, target.annotation_index))
        {
            return Err(fail(
                "widget occurrence does not belong to this field/revision",
            ));
        }
        if widget.expected_display.len() > MAX_TEXT_BYTES
            || widget.replacement_display.len() > MAX_TEXT_BYTES
        {
            return Err(unsupported("widget display budget exceeded"));
        }
        if original_displays.text(target.page, target.annotation)? != widget.expected_display {
            return Err(fail("widget display compare-and-swap failed"));
        }
        let key = serde_json::to_vec(target).map_err(|_| fail("widget target serialization"))?;
        if let std::collections::btree_map::Entry::Vacant(e) = page_targets.entry(target.page) {
            let mut keys = BTreeSet::new();
            for scope in discover_scopes(&engine, target.page, &revision)? {
                crate::cancel::check_current_cancel("widget original target index")?;
                let key = serde_json::to_vec(super::target(&scope)?)
                    .map_err(|_| fail("widget source target serialization"))?;
                target_bytes = target_bytes.saturating_add(key.len());
                if target_bytes > 16 * 1024 * 1024 {
                    return Err(unsupported("widget source target index budget exceeded"));
                }
                keys.insert(key);
            }
            e.insert(keys);
        }
        if !page_targets[&target.page].contains(&key) {
            return Err(fail("widget source occurrence is not exact"));
        }
    }
    // These caches belong only to the immutable input. Do not retain their
    // memory or provenance through private mutations of the document.
    drop(page_targets);
    drop(original_displays);
    let mut output = input.to_vec();
    let mut reports = Vec::with_capacity(request.widgets.len());
    let mut applied_logical_owners = BTreeSet::new();
    for widget in &request.widgets {
        crate::cancel::check_current_cancel("widget appearance transaction")?;
        let current = ContentEngine::open_bytes(output.clone())?;
        let scope = output_scope(&current, &widget.appearance.target)?;
        let mut edit = widget.appearance.clone();
        edit.target = target(&scope)?.clone();
        edit.target.input_sha256 = format!("{:x}", Sha256::digest(&output));
        for update in &mut edit.tagged_clone.actual_text_updates {
            if applied_logical_owners.contains(&update.element) {
                let approved = &logical_updates[&update.element].1;
                let owner = annotation(current.document().reader(), update.element)?;
                if text(resolved(current.document().reader(), &owner, "ActualText")?)? != *approved
                {
                    return Err(fail(
                        "staged logical owner differs from the already approved update",
                    ));
                }
                // Earlier widgets already applied this exact batch decision.
                // Rebase the CAS, not its owner or approved replacement.
                update.expected_text = approved.clone();
            }
        }
        let (next, report) = edit_appearance_text_inner(&output, &edit, font_bytes, true)?;
        applied_logical_owners.extend(
            edit.tagged_clone
                .actual_text_updates
                .iter()
                .map(|update| update.element),
        );
        output = next;
        reports.push(report);
    }
    let current = ContentEngine::open_bytes(output.clone())?;
    let reader = current.document().reader();
    let mut changes = BTreeMap::<ObjectRef, PdfDictionary>::new();
    let mut field = annotation(reader, request.target.field)?;
    field.insert(
        "V",
        crate::annotation_identity::text_string(&request.replacement_value),
    );
    if request.update_default_value {
        field.insert(
            "DV",
            crate::annotation_identity::text_string(&request.replacement_value),
        );
    }
    if request.discard_rich_text {
        field.remove("RV");
        field.remove("DS");
        field.insert("Ff", PdfObject::Integer(source.flags & !(1 << 25)));
    }
    changes.insert(request.target.field, field);
    for id in source.widgets.keys() {
        let entry = changes.entry(*id).or_insert(annotation(reader, *id)?);
        if *id != request.target.field {
            // /V and /DV belong to the terminal field, not a conflicting
            // widget-local shadow. Preserve unrelated widget dictionaries.
            if ["V", "DV", "Ff", "MaxLen"]
                .iter()
                .any(|key| entry.contains_key(key))
            {
                return Err(fail(
                    "pure widget has competing field values or constraints",
                ));
            }
        }
        if request.discard_rich_text {
            entry.remove("RV");
            entry.remove("DS");
        }
    }
    let default_report = apply_defaults(&current, request, &source, &reports, &mut changes)?;
    let updates = changes
        .iter()
        .map(|(id, dict)| IncrementalObject {
            number: id.0,
            generation: id.1,
            object: PdfObject::Dictionary(dict.clone()),
        })
        .collect();
    output = write_incremental_update(reader, updates)?;
    // Individual scoped writers validate private staged appearances. Publish
    // only their final definition delta against the original reader so the
    // saved PDF has one transaction revision, not partially updated field/AP
    // states as intermediate historical revisions.
    let staged = ContentEngine::open_bytes(output)?;
    let staged_reader = staged.document().reader();
    let original_reader = engine.document().reader();
    if staged_reader.root_reference() != original_reader.root_reference() {
        return Err(fail("widget staging changed catalog identity"));
    }
    let mut final_updates = Vec::new();
    for (number, generation) in staged_reader.incremental_definition_ids_since(original_reader)? {
        crate::cancel::check_current_cancel("widget final definition transaction")?;
        final_updates.push(IncrementalObject {
            number,
            generation,
            object: staged_reader.get_object(number, generation)?,
        });
    }
    output = write_incremental_update(original_reader, final_updates)?;
    let saved = ContentEngine::open_bytes(output.clone())?;
    let expected_prev = i64::try_from(original_reader.startxref_offset())
        .map_err(|_| fail("original xref offset exceeds the PDF integer budget"))?;
    if !output.starts_with(input)
        || saved.document().reader().trailer().get_integer("Prev") != Some(expected_prev)
    {
        return Err(fail(
            "field transaction did not publish as one incremental revision",
        ));
    }
    for (id, expected) in &changes {
        if annotation(saved.document().reader(), *id)? != *expected {
            return Err(fail(
                "saved field/default/widget dictionary differs from its staged mutation",
            ));
        }
    }
    for (id, (_, replacement)) in &logical_updates {
        let owner = annotation(saved.document().reader(), *id)?;
        if text(resolved(saved.document().reader(), &owner, "ActualText")?)? != *replacement {
            return Err(fail(
                "saved field logical owner differs from its approved replacement",
            ));
        }
    }
    let mut after_target = request.target.clone();
    after_target.input_sha256 = format!("{:x}", Sha256::digest(&output));
    let after = Binding::read(&saved, &after_target)?;
    if after.widgets != source.widgets || after.value != request.replacement_value {
        return Err(fail(
            "saved field value or ownership differs from the approved transaction",
        ));
    }
    let mut saved_displays = PageDisplays::new(&saved);
    for (widget, report) in request.widgets.iter().zip(&mut reports) {
        if saved_displays.text(
            widget.appearance.target.page,
            widget.appearance.target.annotation,
        )? != widget.replacement_display
        {
            return Err(fail(
                "saved widget display differs from its approved mapping",
            ));
        }
        let scope = output_scope(&saved, &widget.appearance.target)?;
        report.target_after = target(&scope)?.clone();
        report.target_after.input_sha256 = after_target.input_sha256.clone();
    }
    if engine
        .document()
        .get_catalog()?
        .contains_key("StructTreeRoot")
    {
        crate::tagged_structure::validate_parent_tree(&output)?;
    }
    let widget_pages = source
        .widgets
        .values()
        .map(|(page, _)| *page)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let structural_invalidation = request.widgets.iter().any(|widget| {
        widget.appearance.tagged_clone.policy
            != crate::tagged_structure::stream_clones::TaggedClonePolicy::Reject
    });
    let affected_pages = if structural_invalidation {
        // Ancestor logical text and owner-tree rebuilding can affect semantics
        // outside the widget pages. Do not publish an unjustified minimal set.
        (1..=saved.page_count()?).collect()
    } else {
        widget_pages.clone()
    };
    Ok((output,WidgetTextEditReport{schema_version:"advanced_editing.widget-text-transaction.v1".into(),target_before:request.target.clone(),target_after:after_target,
        value_before:source.value,value_after:after.value,widgets:reports,all_widget_displays_verified:true,field_ownership_verified:true,single_published_revision:true,
        native_reports_scope:"native_edit metrics describe private staging; target_after is rebound to the final published revision".into(),
        default_appearance:default_report,default_value_updated:request.update_default_value,rich_text_discarded:request.discard_rich_text,
        actions_preserved_without_execution:actions,widget_pages,affected_pages,
        affected_pages_scope:if structural_invalidation {"conservative_document_wide_for_structural_owners"} else {"physical_widget_pages"}.into(),
        limits:vec!["native source appearances and terminal field value publish in one revision; original historical bytes remain, not redaction".into(),
            "future viewer editing and retained actions may regenerate appearances; source/selected defaults and font coverage remain explicit".into(),
            "source implementation only; rendering, bindings, interoperability and corpus qualification pending".into()]}))
}

fn apply_defaults(
    engine: &ContentEngine,
    request: &WidgetTextEditRequest,
    source: &Binding,
    reports: &[AppearanceTextEditReport],
    changes: &mut BTreeMap<ObjectRef, PdfDictionary>,
) -> Result<Value> {
    let WidgetDefaultAppearance::FromEditedAppearance {
        widget,
        font_resource,
        font_size,
        rgb,
    } = &request.default_appearance
    else {
        return Ok(
            json!({"policy":"preserve_source_defaults","viewer_future_edit_style_preserved":true,"generated_font_does_not_implicitly_replace_defaults":true}),
        );
    };
    if !font_size.is_finite()
        || *font_size < 0.0
        || (*font_size > 0.0 && *font_size < 0.01)
        || *font_size > 1000.0
        || !rgb.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v))
    {
        return Err(fail("invalid default appearance size or colour"));
    }
    let report = reports
        .iter()
        .find(|report| report.target_after.annotation == *widget)
        .ok_or_else(|| fail("default font widget is not in this field transaction"))?;
    let scope = output_scope(engine, &report.target_after)?;
    let reader = engine.document().reader();
    let fonts = dict(
        reader,
        scope
            .source_page
            .resources
            .get("Font")
            .ok_or_else(|| fail("edited source has no font resources"))?,
    )?;
    let font = fonts
        .get(font_resource)
        .cloned()
        .ok_or_else(|| fail("default font name is absent from the edited source"))?;
    let font_dict = dict(reader, &font)?;
    if font_dict.get_name("Type") != Some("Font") {
        return Err(fail("default font resource is not a font"));
    }
    // Tag cloning may have rewritten a direct StructTreeRoot in the catalog.
    // Always compose with the current staged revision, never the input catalog.
    let mut catalog = engine.document().get_catalog()?;
    let form_value = catalog
        .get("AcroForm")
        .ok_or_else(|| fail("missing current AcroForm"))?;
    let form_ref = form_value.as_reference();
    let mut form = dict(reader, form_value)?;
    let mut resources = form
        .get("DR")
        .map(|v| dict(reader, v))
        .transpose()?
        .unwrap_or_else(PdfDictionary::empty);
    let mut fonts = resources
        .get("Font")
        .map(|v| dict(reader, v))
        .transpose()?
        .unwrap_or_else(PdfDictionary::empty);
    let mut name = None;
    for index in 0..1024 {
        let candidate = format!(
            "WFField{}_{}_{}",
            request.target.field.0, request.target.field.1, index
        );
        if fonts.get(&candidate).is_none() || fonts.get(&candidate) == Some(&font) {
            name = Some(candidate);
            break;
        }
    }
    let name =
        name.ok_or_else(|| unsupported("default font resource namespace budget exhausted"))?;
    fonts.insert(name.clone(), font.clone());
    resources.insert("Font", PdfObject::Dictionary(fonts));
    form.insert("DR", PdfObject::Dictionary(resources));
    let program = format!(
        "/{name} {} Tf {} {} {} rg",
        fmt_num(*font_size),
        fmt_num(rgb[0]),
        fmt_num(rgb[1]),
        fmt_num(rgb[2])
    );
    changes
        .get_mut(&request.target.field)
        .ok_or_else(|| fail("missing staged field"))?
        .insert("DA", PdfObject::String(program.as_bytes().to_vec()));
    for id in source
        .widgets
        .keys()
        .filter(|id| **id != request.target.field)
    {
        changes
            .get_mut(id)
            .ok_or_else(|| fail("missing staged widget"))?
            .remove("DA");
    }
    if let Some(id) = form_ref {
        changes.insert(id, form);
    } else {
        catalog.insert("AcroForm", PdfObject::Dictionary(form));
        let id = reader
            .root_reference()
            .ok_or_else(|| fail("missing catalog identity"))?;
        changes.insert(id, catalog);
    }
    Ok(
        json!({"policy":"from_edited_appearance","source_widget":widget,"source_font_resource":font_resource,"field_font_resource":name,"font_reference":font.as_reference(),
        "default_appearance":program,"automatic_viewer_font_size":*font_size==0.0,"widget_default_overrides_removed":true,"future_glyph_coverage_not_guaranteed":true}),
    )
}
