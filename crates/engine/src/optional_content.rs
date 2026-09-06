use std::collections::{BTreeSet, HashMap, HashSet};

use serde::Serialize;

use crate::document::PdfDocument;
use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;

const DEFAULT_FINGERPRINT: &str = "ocg:none";

#[derive(Clone, Debug, Serialize)]
pub struct OptionalContentReport {
    pub status: String,
    pub ocproperties_present: bool,
    pub active_configuration: String,
    pub visibility_fingerprint: String,
    pub layers: Vec<OptionalContentLayerReport>,
    pub membership_dictionaries: Vec<OptionalContentMembershipReport>,
    pub order_tree_entries: Vec<String>,
    pub radio_groups: Vec<Vec<String>>,
    pub locked_layers: Vec<String>,
    pub supported_visibility_policies: Vec<String>,
    pub malformed_policy: String,
    pub diagnostics: Vec<String>,
}

impl Default for OptionalContentReport {
    fn default() -> Self {
        Self {
            status: "not_present".to_string(),
            ocproperties_present: false,
            active_configuration: "none".to_string(),
            visibility_fingerprint: DEFAULT_FINGERPRINT.to_string(),
            layers: Vec::new(),
            membership_dictionaries: Vec::new(),
            order_tree_entries: Vec::new(),
            radio_groups: Vec::new(),
            locked_layers: Vec::new(),
            supported_visibility_policies: vec![
                "BaseState".to_string(),
                "ON".to_string(),
                "OFF".to_string(),
                "Intent".to_string(),
                "Usage/View".to_string(),
                "OCMD/AnyOn".to_string(),
                "OCMD/AllOn".to_string(),
                "OCMD/AnyOff".to_string(),
                "OCMD/AllOff".to_string(),
            ],
            malformed_policy: "fail_closed_with_typed_error_for_render_visibility".to_string(),
            diagnostics: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct OptionalContentLayerReport {
    pub id: String,
    pub name: String,
    pub default_state: bool,
    pub base_state: String,
    pub explicit_state_source: String,
    pub intent: Vec<String>,
    pub usage_view_state: Option<String>,
    pub usage_print_state: Option<String>,
    pub usage_export_state: Option<String>,
    pub locked: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct OptionalContentMembershipReport {
    pub id: String,
    pub policy: String,
    pub members: Vec<String>,
    pub visible: bool,
}

#[derive(Clone, Debug)]
struct LayerState {
    id: String,
    name: String,
    visible: bool,
    base_state: String,
    explicit_state_source: String,
    intent: Vec<String>,
    usage_view_state: Option<String>,
    usage_print_state: Option<String>,
    usage_export_state: Option<String>,
    locked: bool,
}

#[derive(Clone, Debug)]
enum OptionalContentConfigSelector {
    Default,
    Name(String),
    Index(usize),
}

#[derive(Clone, Debug)]
pub struct OptionalContentContext {
    states: HashMap<String, bool>,
    report: OptionalContentReport,
    strict_error: Option<String>,
}

impl OptionalContentContext {
    pub fn absent() -> Self {
        Self {
            states: HashMap::new(),
            report: OptionalContentReport::default(),
            strict_error: None,
        }
    }

    pub fn from_document(document: &PdfDocument) -> Self {
        Self::from_document_with_config_selector(document, OptionalContentConfigSelector::Default)
    }

    pub fn from_document_for_state(
        document: &PdfDocument,
        state_id: &str,
    ) -> std::result::Result<Self, String> {
        let requested = state_id.trim();
        let default_context = Self::from_document(document);
        if requested.is_empty()
            || requested == "default"
            || requested == "ocg:default"
            || requested == default_context.visibility_fingerprint()
        {
            return Ok(default_context);
        }

        let selector = if let Some(name) = requested.strip_prefix("ocg:config:") {
            if name.trim().is_empty() {
                return Err(
                    "render contract optional_content ocg:config selector is empty".to_string(),
                );
            }
            OptionalContentConfigSelector::Name(name.trim().to_string())
        } else if let Some(index) = requested.strip_prefix("ocg:config-index:") {
            let index = index.trim().parse::<usize>().map_err(|_| {
                format!(
                    "render contract optional_content config index '{index}' is not a non-negative integer"
                )
            })?;
            OptionalContentConfigSelector::Index(index)
        } else {
            return Err(format!(
                "unsupported render contract optional_content state '{requested}'; expected default, ocg:default, the active fingerprint, ocg:config:<Name>, or ocg:config-index:<n>"
            ));
        };

        ensure_config_selector_available(document, &selector)?;
        Ok(Self::from_document_with_config_selector(document, selector))
    }

    fn from_document_with_config_selector(
        document: &PdfDocument,
        selector: OptionalContentConfigSelector,
    ) -> Self {
        let mut report = OptionalContentReport::default();
        let mut strict_error = None;
        let reader = document.reader();
        let catalog = match document.get_catalog() {
            Ok(catalog) => catalog,
            Err(err) => {
                report.status = "catalog_unavailable_strict_render_visibility".to_string();
                record_strict_error(
                    &mut report,
                    &mut strict_error,
                    format!("catalog unavailable: {err}"),
                );
                return Self {
                    states: HashMap::new(),
                    report,
                    strict_error,
                };
            }
        };

        let Some(ocprops_obj) = catalog.get("OCProperties") else {
            return Self {
                states: HashMap::new(),
                report,
                strict_error,
            };
        };
        report.ocproperties_present = true;

        let Some(ocprops) = resolve_dict(ocprops_obj, reader) else {
            report.status = "malformed_ocproperties_strict_render_visibility".to_string();
            record_strict_error(
                &mut report,
                &mut strict_error,
                "/OCProperties did not resolve to a dictionary",
            );
            return Self {
                states: HashMap::new(),
                report,
                strict_error,
            };
        };

        let config =
            selected_config_dict(&ocprops, reader, &selector, &mut report, &mut strict_error);
        report.active_configuration = config
            .get("Name")
            .and_then(pdf_text_or_name)
            .unwrap_or_else(|| "default".to_string());
        let base_state = match config.get("BaseState") {
            Some(PdfObject::Name(name)) if matches!(name.as_str(), "ON" | "OFF" | "Unchanged") => {
                name.to_ascii_uppercase()
            }
            Some(PdfObject::Name(name)) => {
                record_strict_error(
                    &mut report,
                    &mut strict_error,
                    format!("/OCProperties /D /BaseState has unsupported name /{name}"),
                );
                "ON".to_string()
            }
            Some(object) => {
                record_strict_error(
                    &mut report,
                    &mut strict_error,
                    format!(
                        "/OCProperties /D /BaseState resolved to {}, expected Name",
                        object.variant_name()
                    ),
                );
                "ON".to_string()
            }
            None => "ON".to_string(),
        };
        let base_visible = base_state != "OFF";

        let locked = optional_object_id_set(
            &config,
            "Locked",
            &mut report,
            &mut strict_error,
            "/OCProperties /D",
        );
        report.locked_layers = locked.iter().cloned().collect();
        report.radio_groups = optional_radio_groups(&config, &mut report, &mut strict_error);
        report.order_tree_entries = flatten_order_tree(config.get("Order"));

        let on = optional_object_id_set(
            &config,
            "ON",
            &mut report,
            &mut strict_error,
            "/OCProperties /D",
        );
        let off = optional_object_id_set(
            &config,
            "OFF",
            &mut report,
            &mut strict_error,
            "/OCProperties /D",
        );
        let config_intents = intent_names(config.get("Intent"));

        let mut states = HashMap::new();
        let mut layers = Vec::new();
        let mut seen = HashSet::new();
        match ocprops.get("OCGs") {
            Some(PdfObject::Array(ocgs)) => {
                for layer_obj in ocgs {
                    let id = object_id(layer_obj);
                    if !seen.insert(id.clone()) {
                        continue;
                    }
                    let Some(layer_dict) = resolve_dict(layer_obj, reader) else {
                        record_strict_error(
                            &mut report,
                            &mut strict_error,
                            format!("OCG {id} did not resolve to a dictionary"),
                        );
                        continue;
                    };
                    if layer_dict.get_name("Type") != Some("OCG") {
                        record_strict_error(
                            &mut report,
                            &mut strict_error,
                            format!("OCG {id} has non-OCG /Type"),
                        );
                    }

                    let name = layer_dict
                        .get("Name")
                        .and_then(pdf_text_or_name)
                        .unwrap_or_else(|| id.clone());
                    let layer_intents = intent_names(layer_dict.get("Intent"));
                    let usage_view_state = usage_state(&layer_dict, "View");
                    let usage_print_state = usage_state(&layer_dict, "Print");
                    let usage_export_state = usage_state(&layer_dict, "Export");
                    let mut visible = base_visible;
                    let mut source = format!("BaseState/{base_state}");

                    if !config_intents.is_empty()
                        && !layer_intents.is_empty()
                        && !layer_intents
                            .iter()
                            .any(|intent| config_intents.contains(intent))
                    {
                        visible = false;
                        source = "Intent/mismatch".to_string();
                    }

                    if matches!(usage_view_state.as_deref(), Some("OFF")) {
                        visible = false;
                        source = "Usage/View/OFF".to_string();
                    } else if matches!(usage_view_state.as_deref(), Some("ON")) {
                        visible = true;
                        source = "Usage/View/ON".to_string();
                    }

                    if on.contains(&id) {
                        visible = true;
                        source = "ON".to_string();
                    }
                    if off.contains(&id) {
                        visible = false;
                        source = "OFF".to_string();
                    }

                    states.insert(id.clone(), visible);
                    layers.push(LayerState {
                        id,
                        name,
                        visible,
                        base_state: base_state.clone(),
                        explicit_state_source: source,
                        intent: layer_intents,
                        usage_view_state,
                        usage_print_state,
                        usage_export_state,
                        locked: false,
                    });
                }
            }
            Some(object) => record_strict_error(
                &mut report,
                &mut strict_error,
                format!(
                    "/OCProperties /OCGs resolved to {}, expected Array",
                    object.variant_name()
                ),
            ),
            None => record_strict_error(
                &mut report,
                &mut strict_error,
                "/OCProperties missing required /OCGs array",
            ),
        }

        for layer in &mut layers {
            layer.locked = locked.contains(&layer.id);
        }

        report.layers = layers
            .into_iter()
            .map(|layer| OptionalContentLayerReport {
                id: layer.id,
                name: layer.name,
                default_state: layer.visible,
                base_state: layer.base_state,
                explicit_state_source: layer.explicit_state_source,
                intent: layer.intent,
                usage_view_state: layer.usage_view_state,
                usage_print_state: layer.usage_print_state,
                usage_export_state: layer.usage_export_state,
                locked: layer.locked,
            })
            .collect();
        report.status = if report.layers.is_empty() {
            "parsed_no_layers_strict_render_visibility".to_string()
        } else if matches!(selector, OptionalContentConfigSelector::Default) {
            "parsed_default_view_configuration".to_string()
        } else {
            "parsed_selected_view_configuration".to_string()
        };
        report.visibility_fingerprint = fingerprint_for_states(&states);

        Self {
            states,
            report,
            strict_error,
        }
    }

    pub fn report(&self) -> &OptionalContentReport {
        &self.report
    }

    pub fn visibility_fingerprint(&self) -> &str {
        &self.report.visibility_fingerprint
    }

    pub fn is_resource_visible(
        &self,
        name: &str,
        properties: &HashMap<String, PdfObject>,
        reader: &PdfReader,
    ) -> bool {
        self.is_resource_visible_strict(name, properties, reader, "optional-content resource")
            .unwrap_or(true)
    }

    pub fn is_object_visible(&self, object: Option<&PdfObject>, reader: &PdfReader) -> bool {
        self.is_object_visible_strict(object, reader, "optional-content object")
            .unwrap_or(true)
    }

    pub fn is_resource_visible_strict(
        &self,
        name: &str,
        properties: &HashMap<String, PdfObject>,
        reader: &PdfReader,
        label: &str,
    ) -> Result<bool, String> {
        let Some(object) = properties.get(name) else {
            return Err(format!("{label} property resource /{name} is missing"));
        };
        self.is_object_visible_strict(
            Some(object),
            reader,
            &format!("{label} property resource /{name}"),
        )
    }

    pub fn is_object_visible_strict(
        &self,
        object: Option<&PdfObject>,
        reader: &PdfReader,
        label: &str,
    ) -> Result<bool, String> {
        let Some(object) = object else {
            return Ok(true);
        };
        if let Some(err) = &self.strict_error {
            return Err(format!(
                "{label} optional-content configuration malformed: {err}"
            ));
        }
        self.is_object_visible_inner(object, reader, &mut HashSet::new())
            .map_err(|err| format!("{label} {err}"))
    }

    fn is_object_visible_inner(
        &self,
        object: &PdfObject,
        reader: &PdfReader,
        visiting: &mut HashSet<String>,
    ) -> Result<bool, String> {
        let id = object_id(object);
        if !visiting.insert(id.clone()) {
            return Err(format!("has cyclic optional-content reference {id}"));
        }
        let resolved = reader
            .resolve(object.clone())
            .map_err(|err| format!("{id} failed to resolve: {err}"))?;
        let result = match &resolved {
            PdfObject::Dictionary(dict) => match dict.get_name("Type") {
                Some("OCG") => self
                    .states
                    .get(&id)
                    .copied()
                    .or_else(|| {
                        let direct_id = object_id(&PdfObject::Dictionary(dict.clone()));
                        self.states.get(&direct_id).copied()
                    })
                    .ok_or_else(|| format!("OCG {id} has no configured visibility state")),
                Some("OCMD") => self.evaluate_ocmd(dict, reader, visiting),
                Some(other) => Err(format!(
                    "{id} has unsupported optional-content /Type /{other}"
                )),
                None => Err(format!("{id} missing required optional-content /Type")),
            },
            other => Err(format!(
                "{id} resolved to {}, expected optional-content dictionary",
                other.variant_name()
            )),
        };
        visiting.remove(&id);
        result
    }

    fn evaluate_ocmd(
        &self,
        dict: &PdfDictionary,
        reader: &PdfReader,
        visiting: &mut HashSet<String>,
    ) -> Result<bool, String> {
        let policy = match dict.get("P") {
            Some(PdfObject::Name(name)) => name.as_str(),
            Some(object) => {
                return Err(format!(
                    "OCMD /P resolved to {}, expected Name",
                    object.variant_name()
                ))
            }
            None => "AnyOn",
        };
        let mut states = Vec::new();
        let Some(ocgs) = dict.get("OCGs") else {
            return Err("OCMD missing required /OCGs".to_string());
        };
        match ocgs {
            PdfObject::Array(items) => {
                if items.is_empty() {
                    return Err("OCMD /OCGs array is empty".to_string());
                }
                for item in items {
                    states.push(self.is_object_visible_inner(item, reader, visiting)?);
                }
            }
            other => states.push(self.is_object_visible_inner(other, reader, visiting)?),
        }
        let visible = match policy {
            "AllOn" => states.iter().all(|state| *state),
            "AnyOff" => states.iter().any(|state| !*state),
            "AllOff" => states.iter().all(|state| !*state),
            "AnyOn" => states.iter().any(|state| *state),
            other => {
                return Err(format!(
                    "OCMD has unsupported optional-content membership policy /{other}"
                ))
            }
        };
        Ok(visible)
    }
}

fn record_strict_error(
    report: &mut OptionalContentReport,
    strict_error: &mut Option<String>,
    message: impl Into<String>,
) {
    let message = message.into();
    report.diagnostics.push(message.clone());
    if strict_error.is_none() {
        *strict_error = Some(message);
    }
}

fn ensure_config_selector_available(
    document: &PdfDocument,
    selector: &OptionalContentConfigSelector,
) -> std::result::Result<(), String> {
    if matches!(selector, OptionalContentConfigSelector::Default) {
        return Ok(());
    }
    let reader = document.reader();
    let catalog = document
        .get_catalog()
        .map_err(|err| format!("catalog unavailable while selecting optional content: {err}"))?;
    let ocprops_obj = catalog.get("OCProperties").ok_or_else(|| {
        "render contract selected optional_content but document has no /OCProperties".to_string()
    })?;
    let ocprops = resolve_dict(ocprops_obj, reader)
        .ok_or_else(|| "/OCProperties did not resolve to a dictionary".to_string())?;
    let configs = match ocprops.get("Configs") {
        Some(PdfObject::Array(configs)) => configs,
        Some(object) => {
            return Err(format!(
                "/OCProperties /Configs resolved to {}, expected Array",
                object.variant_name()
            ))
        }
        None => return Err(
            "render contract selected optional_content but document has no /OCProperties /Configs"
                .to_string(),
        ),
    };
    match selector {
        OptionalContentConfigSelector::Default => Ok(()),
        OptionalContentConfigSelector::Index(index) => configs
            .get(*index)
            .map(|_| ())
            .ok_or_else(|| format!("optional-content configuration index {index} not found")),
        OptionalContentConfigSelector::Name(name) => {
            for config in configs {
                let Some(config_dict) = resolve_dict(config, reader) else {
                    continue;
                };
                if config_dict
                    .get("Name")
                    .and_then(pdf_text_or_name)
                    .as_deref()
                    == Some(name.as_str())
                {
                    return Ok(());
                }
            }
            Err(format!(
                "optional-content configuration named '{name}' not found"
            ))
        }
    }
}

fn selected_config_dict(
    ocprops: &PdfDictionary,
    reader: &PdfReader,
    selector: &OptionalContentConfigSelector,
    report: &mut OptionalContentReport,
    strict_error: &mut Option<String>,
) -> PdfDictionary {
    match selector {
        OptionalContentConfigSelector::Default => {
            default_config_dict(ocprops, reader, report, strict_error)
        }
        OptionalContentConfigSelector::Index(index) => {
            match config_dict_by_index(ocprops, reader, *index) {
                Ok(config) => config,
                Err(err) => {
                    record_strict_error(report, strict_error, err);
                    PdfDictionary::empty()
                }
            }
        }
        OptionalContentConfigSelector::Name(name) => {
            match config_dict_by_name(ocprops, reader, name) {
                Ok(config) => config,
                Err(err) => {
                    record_strict_error(report, strict_error, err);
                    PdfDictionary::empty()
                }
            }
        }
    }
}

fn default_config_dict(
    ocprops: &PdfDictionary,
    reader: &PdfReader,
    report: &mut OptionalContentReport,
    strict_error: &mut Option<String>,
) -> PdfDictionary {
    match ocprops.get("D") {
        Some(object) => match resolve_dict(object, reader) {
            Some(dict) => dict,
            None => {
                record_strict_error(
                    report,
                    strict_error,
                    format!(
                        "/OCProperties /D resolved to {}, expected Dictionary",
                        object.variant_name()
                    ),
                );
                PdfDictionary::empty()
            }
        },
        None => PdfDictionary::empty(),
    }
}

fn config_dict_by_index(
    ocprops: &PdfDictionary,
    reader: &PdfReader,
    index: usize,
) -> std::result::Result<PdfDictionary, String> {
    let configs = optional_content_configs(ocprops)?;
    let Some(config) = configs.get(index) else {
        return Err(format!(
            "optional-content configuration index {index} not found"
        ));
    };
    resolve_dict(config, reader).ok_or_else(|| {
        format!(
            "optional-content configuration index {index} resolved to {}, expected Dictionary",
            config.variant_name()
        )
    })
}

fn config_dict_by_name(
    ocprops: &PdfDictionary,
    reader: &PdfReader,
    name: &str,
) -> std::result::Result<PdfDictionary, String> {
    let configs = optional_content_configs(ocprops)?;
    for config in configs {
        let Some(config_dict) = resolve_dict(config, reader) else {
            continue;
        };
        if config_dict
            .get("Name")
            .and_then(pdf_text_or_name)
            .as_deref()
            == Some(name)
        {
            return Ok(config_dict);
        }
    }
    Err(format!(
        "optional-content configuration named '{name}' not found"
    ))
}

fn optional_content_configs(ocprops: &PdfDictionary) -> std::result::Result<&[PdfObject], String> {
    match ocprops.get("Configs") {
        Some(PdfObject::Array(configs)) => Ok(configs),
        Some(object) => Err(format!(
            "/OCProperties /Configs resolved to {}, expected Array",
            object.variant_name()
        )),
        None => Err("/OCProperties missing /Configs array".to_string()),
    }
}

fn optional_object_id_set(
    dict: &PdfDictionary,
    key: &str,
    report: &mut OptionalContentReport,
    strict_error: &mut Option<String>,
    owner: &str,
) -> BTreeSet<String> {
    match dict.get(key) {
        Some(PdfObject::Array(items)) => object_id_set(items),
        Some(object) => {
            record_strict_error(
                report,
                strict_error,
                format!(
                    "{owner} /{key} resolved to {}, expected Array",
                    object.variant_name()
                ),
            );
            BTreeSet::new()
        }
        None => BTreeSet::new(),
    }
}

fn optional_radio_groups(
    dict: &PdfDictionary,
    report: &mut OptionalContentReport,
    strict_error: &mut Option<String>,
) -> Vec<Vec<String>> {
    match dict.get("RBGroups") {
        Some(PdfObject::Array(items)) => parse_radio_groups(items),
        Some(object) => {
            record_strict_error(
                report,
                strict_error,
                format!(
                    "/OCProperties /D /RBGroups resolved to {}, expected Array",
                    object.variant_name()
                ),
            );
            Vec::new()
        }
        None => Vec::new(),
    }
}

fn resolve_dict(object: &PdfObject, reader: &PdfReader) -> Option<PdfDictionary> {
    match reader.resolve(object.clone()).ok()? {
        PdfObject::Dictionary(dict) => Some(dict),
        PdfObject::Stream { dict, .. } => Some(dict),
        _ => None,
    }
}

fn object_id(object: &PdfObject) -> String {
    match object {
        PdfObject::Reference { number, generation } => format!("{number}:{generation}"),
        PdfObject::Dictionary(dict) | PdfObject::Stream { dict, .. } => dict
            .get("Name")
            .and_then(pdf_text_or_name)
            .or_else(|| dict.get_name("Type").map(|name| format!("direct:{name}")))
            .unwrap_or_else(|| "direct:dictionary".to_string()),
        PdfObject::Name(name) => format!("name:{name}"),
        _ => format!("direct:{}", object.variant_name()),
    }
}

fn object_id_set(items: &[PdfObject]) -> BTreeSet<String> {
    items.iter().map(object_id).collect()
}

fn pdf_text_or_name(object: &PdfObject) -> Option<String> {
    match object {
        PdfObject::Name(name) => Some(name.clone()),
        PdfObject::String(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
        _ => None,
    }
}

fn intent_names(object: Option<&PdfObject>) -> Vec<String> {
    match object {
        Some(PdfObject::Name(name)) => vec![name.clone()],
        Some(PdfObject::Array(items)) => items
            .iter()
            .filter_map(PdfObject::as_name)
            .map(ToString::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

fn usage_state(layer: &PdfDictionary, usage_key: &str) -> Option<String> {
    let usage = layer.get_dict("Usage")?;
    let dict = usage.get_dict(usage_key)?;
    dict.get_name(&format!("{usage_key}State"))
        .or_else(|| dict.get_name("State"))
        .map(ToString::to_string)
}

fn parse_radio_groups(items: &[PdfObject]) -> Vec<Vec<String>> {
    items
        .iter()
        .filter_map(PdfObject::as_array)
        .map(|group| group.iter().map(object_id).collect())
        .collect()
}

fn flatten_order_tree(object: Option<&PdfObject>) -> Vec<String> {
    let mut out = Vec::new();
    flatten_order_tree_inner(object, &mut out);
    out
}

fn flatten_order_tree_inner(object: Option<&PdfObject>, out: &mut Vec<String>) {
    match object {
        Some(PdfObject::Array(items)) => {
            for item in items {
                flatten_order_tree_inner(Some(item), out);
            }
        }
        Some(PdfObject::String(bytes)) => out.push(String::from_utf8_lossy(bytes).into_owned()),
        Some(PdfObject::Name(name)) => out.push(name.clone()),
        Some(PdfObject::Reference { .. }) | Some(PdfObject::Dictionary(_)) => {
            out.push(object_id(object.unwrap()))
        }
        _ => {}
    }
}

fn fingerprint_for_states(states: &HashMap<String, bool>) -> String {
    if states.is_empty() {
        return DEFAULT_FINGERPRINT.to_string();
    }
    let mut entries: Vec<_> = states.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    let mut fingerprint = String::from("ocg:view:");
    for (index, (id, visible)) in entries.into_iter().enumerate() {
        if index > 0 {
            fingerprint.push('|');
        }
        fingerprint.push_str(id);
        fingerprint.push('=');
        fingerprint.push_str(if *visible { "1" } else { "0" });
    }
    fingerprint
}
