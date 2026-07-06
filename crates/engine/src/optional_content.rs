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
            malformed_policy: "fail_open_with_diagnostic_for_unknown_or_malformed_optional_content"
                .to_string(),
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
pub struct OptionalContentContext {
    states: HashMap<String, bool>,
    report: OptionalContentReport,
}

impl OptionalContentContext {
    pub fn absent() -> Self {
        Self {
            states: HashMap::new(),
            report: OptionalContentReport::default(),
        }
    }

    pub fn from_document(document: &PdfDocument) -> Self {
        let mut report = OptionalContentReport::default();
        let reader = document.reader();
        let catalog = match document.get_catalog() {
            Ok(catalog) => catalog,
            Err(err) => {
                report.status = "catalog_unavailable_fail_open".to_string();
                report.diagnostics.push(format!("catalog: {err}"));
                return Self {
                    states: HashMap::new(),
                    report,
                };
            }
        };

        let Some(ocprops_obj) = catalog.get("OCProperties") else {
            return Self {
                states: HashMap::new(),
                report,
            };
        };
        report.ocproperties_present = true;

        let Some(ocprops) = resolve_dict(ocprops_obj, reader) else {
            report.status = "malformed_ocproperties_fail_open".to_string();
            report
                .diagnostics
                .push("/OCProperties did not resolve to a dictionary".to_string());
            return Self {
                states: HashMap::new(),
                report,
            };
        };

        let config = ocprops
            .get_dict("D")
            .cloned()
            .unwrap_or_else(PdfDictionary::empty);
        report.active_configuration = config
            .get("Name")
            .and_then(pdf_text_or_name)
            .unwrap_or_else(|| "default".to_string());
        let base_state = config
            .get_name("BaseState")
            .unwrap_or("ON")
            .to_ascii_uppercase();
        let base_visible = base_state != "OFF";

        let locked = object_id_set(config.get_array("Locked").unwrap_or(&[]));
        report.locked_layers = locked.iter().cloned().collect();
        report.radio_groups = parse_radio_groups(config.get_array("RBGroups").unwrap_or(&[]));
        report.order_tree_entries = flatten_order_tree(config.get("Order"));

        let on = object_id_set(config.get_array("ON").unwrap_or(&[]));
        let off = object_id_set(config.get_array("OFF").unwrap_or(&[]));
        let config_intents = intent_names(config.get("Intent"));

        let mut states = HashMap::new();
        let mut layers = Vec::new();
        let mut seen = HashSet::new();
        if let Some(ocgs) = ocprops.get_array("OCGs") {
            for layer_obj in ocgs {
                let id = object_id(layer_obj);
                if !seen.insert(id.clone()) {
                    continue;
                }
                let Some(layer_dict) = resolve_dict(layer_obj, reader) else {
                    report
                        .diagnostics
                        .push(format!("OCG {id} did not resolve to a dictionary"));
                    continue;
                };
                if layer_dict.get_name("Type") != Some("OCG") {
                    report
                        .diagnostics
                        .push(format!("OCG {id} has non-OCG /Type"));
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
            "parsed_no_layers_fail_open".to_string()
        } else {
            "parsed_default_view_configuration".to_string()
        };
        report.visibility_fingerprint = fingerprint_for_states(&states);

        Self { states, report }
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
        properties
            .get(name)
            .map(|object| self.is_object_visible(Some(object), reader))
            .unwrap_or(true)
    }

    pub fn is_object_visible(&self, object: Option<&PdfObject>, reader: &PdfReader) -> bool {
        let Some(object) = object else {
            return true;
        };
        self.is_object_visible_inner(object, reader, &mut HashSet::new())
            .unwrap_or(true)
    }

    fn is_object_visible_inner(
        &self,
        object: &PdfObject,
        reader: &PdfReader,
        visiting: &mut HashSet<String>,
    ) -> Option<bool> {
        let id = object_id(object);
        if !visiting.insert(id.clone()) {
            return Some(true);
        }
        let resolved = match reader.resolve(object.clone()) {
            Ok(resolved) => resolved,
            Err(_) => return Some(true),
        };
        match &resolved {
            PdfObject::Dictionary(dict) => match dict.get_name("Type") {
                Some("OCG") => Some(self.states.get(&id).copied().unwrap_or_else(|| {
                    let direct_id = object_id(&PdfObject::Dictionary(dict.clone()));
                    self.states.get(&direct_id).copied().unwrap_or(true)
                })),
                Some("OCMD") => self.evaluate_ocmd(dict, reader, visiting),
                _ => Some(true),
            },
            _ => Some(true),
        }
    }

    fn evaluate_ocmd(
        &self,
        dict: &PdfDictionary,
        reader: &PdfReader,
        visiting: &mut HashSet<String>,
    ) -> Option<bool> {
        let policy = dict.get_name("P").unwrap_or("AnyOn");
        let mut states = Vec::new();
        if let Some(ocgs) = dict.get("OCGs") {
            match ocgs {
                PdfObject::Array(items) => {
                    for item in items {
                        states.push(self.is_object_visible_inner(item, reader, visiting)?);
                    }
                }
                other => states.push(self.is_object_visible_inner(other, reader, visiting)?),
            }
        }
        if states.is_empty() {
            return Some(true);
        }
        Some(match policy {
            "AllOn" => states.iter().all(|state| *state),
            "AnyOff" => states.iter().any(|state| !*state),
            "AllOff" => states.iter().all(|state| !*state),
            "AnyOn" => states.iter().any(|state| *state),
            _ => true,
        })
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
