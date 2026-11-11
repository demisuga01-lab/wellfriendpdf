//! Interactive XFA form sessions: a host drives the events a viewer's
//! form filler would (PDFium `fpdfsdk/fpdfxfa` + `CXFA_FFDocView`) without
//! a GUI — value edits (`enter`, `change`, `full`, `exit`), clicks
//! (buttons, check boxes, radio buttons with their exclusion groups),
//! arbitrary `execEvent`-style dispatches — each followed by the
//! dependent calculations and validations. The session keeps the script
//! DOM (values, presence, instances, script variables, the JavaScript
//! engine) between calls; rendering re-merges and re-lays out the form
//! with that state, and saving writes the data DOM back into the PDF's
//! `datasets` packet as an incremental update (PDFium
//! `SaveXFADocumentData`, after `preSave` and before `postSave`).
//!
//! Scripts run in the same sandbox as rendering (FormCalc, JavaScript
//! through boa when available), each host action under fresh time,
//! operation and mutation budgets.

use serde::{Deserialize, Serialize};

use crate::cancel::CancelToken;
use crate::error::{Result, WellfriendError};
use crate::images::encoder::ImageEncoder;
use crate::object::{PdfDictionary, PdfObject};
use crate::render::PixelBuffer;
use crate::writer::{write_incremental_update, IncrementalObject};
use crate::ContentEngine;

use super::events::{ActionOutcome, FormScripts, ScriptState};
use super::form::{build_form_model_with_datasets, Template};
use super::render::{has_xfa_entry, paint_form, xfa_sources, XfaFormType, XfaValidationResult};
use super::xml::parse_xml;
use super::{load_xfa, LoadedXfa, XfaLimits};

/// Options of an [`XfaFormSession`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct XfaSessionOptions {
    /// Run the form's scripts (default). Without them events change
    /// nothing but the values the host sets.
    pub run_scripts: bool,
}

impl Default for XfaSessionOptions {
    fn default() -> Self {
        Self { run_scripts: true }
    }
}

/// One choice-list item.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct XfaChoiceItem {
    pub display: String,
    pub save: String,
}

/// A field or exclusion group of a session's form.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct XfaFieldState {
    /// Canonical SOM expression (`xfa[0].form[0].form1[0].name[0]`).
    pub som: String,
    pub name: String,
    /// `field` or `exclGroup`.
    pub kind: String,
    /// UI widget (`textEdit`, `numericEdit`, `checkButton`, `choiceList`,
    /// `button`, `dateTimeEdit`...); empty for exclusion groups.
    pub ui: String,
    /// Value content type (`text`, `decimal`, `date`...).
    pub value_type: String,
    /// Raw (canonical) value; `None` when empty.
    pub value: Option<String>,
    /// `visible`, `invisible`, `hidden` or `inactive`.
    pub presence: String,
    /// `open`, `readOnly`, `protected` or `nonInteractive`.
    pub access: String,
    pub caption: Option<String>,
    pub items: Vec<XfaChoiceItem>,
    /// Check button on/off values.
    pub on_value: Option<String>,
    pub off_value: Option<String>,
    /// Check buttons: whether the value is the on value.
    pub checked: Option<bool>,
    /// Visible, inside visible containers, with open access.
    pub interactive: bool,
}

/// One event the session dispatched.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct XfaFiredEvent {
    pub som: String,
    pub activity: String,
    /// Scripts that handled it (0 when the node has no handler).
    pub scripts: usize,
}

/// What one host action did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct XfaSessionOutcome {
    /// Whether a new value was committed to the target.
    pub committed: bool,
    /// Events dispatched by the action, in order (script-triggered
    /// `execEvent`s run inside them).
    pub events: Vec<XfaFiredEvent>,
    /// `app.alert` / `xfa.host.messageBox` output of the action.
    pub messages: Vec<String>,
    /// Script errors raised by the action.
    pub errors: Vec<String>,
    /// Failed validations of the whole form after the action.
    pub validations: Vec<XfaValidationResult>,
}

/// A stateful XFA form: see the module documentation. Not `Send` (the
/// script engines are single-threaded); hosts serialize access.
pub struct XfaFormSession {
    engine: ContentEngine,
    loaded: LoadedXfa,
    /// `None` only after a panic inside a session call (poisoned).
    state: Option<ScriptState>,
    form_type: XfaFormType,
    /// The document with the current form state painted (rebuilt lazily
    /// after every change).
    view: Option<ContentEngine>,
}

impl ContentEngine {
    /// Opens an interactive XFA form session (scripts enabled).
    pub fn xfa_session(&self) -> Result<XfaFormSession> {
        XfaFormSession::open(self, &XfaSessionOptions::default())
    }

    /// [`ContentEngine::xfa_session`] with explicit options.
    pub fn xfa_session_with_options(&self, options: &XfaSessionOptions) -> Result<XfaFormSession> {
        XfaFormSession::open(self, options)
    }
}

fn no_form() -> WellfriendError {
    WellfriendError::UnsupportedFeature("document has no usable XFA form".into())
}

fn poisoned() -> WellfriendError {
    WellfriendError::invalid_input("XFA session is poisoned by an earlier failure; reopen it")
}

impl XfaFormSession {
    /// Merges the form with its data and runs the document-open events
    /// (`initialize`, `calculate`, validation, `form:ready`, `docReady`).
    pub fn open(engine: &ContentEngine, options: &XfaSessionOptions) -> Result<Self> {
        let document = engine.document();
        if !has_xfa_entry(document) {
            return Err(no_form());
        }
        let loaded = load_xfa(document, &XfaLimits::default(), &CancelToken::none())?;
        let (state, form_type) = {
            let sources = xfa_sources(&loaded).ok_or_else(no_form)?;
            let template = sources.template;
            let tpl = Template::new(template);
            let mut model = build_form_model_with_datasets(
                &tpl,
                template,
                sources.datasets,
                sources.datasets_root,
            )
            .ok_or_else(no_form)?;
            let mut scripts = FormScripts::prepare_session(
                &tpl,
                template,
                &model,
                sources.datasets_root,
                sources.locales.clone(),
                options.run_scripts,
            );
            scripts.before_layout(&tpl, &mut model);
            (scripts.into_state(), sources.form_type)
        };
        Ok(Self {
            engine: engine.clone(),
            loaded,
            state: Some(state),
            form_type,
            view: None,
        })
    }

    /// How the form renders (static foreground or dynamic full pages).
    pub fn form_type(&self) -> XfaFormType {
        self.form_type
    }

    fn state(&self) -> Result<&ScriptState> {
        self.state.as_ref().ok_or_else(poisoned)
    }

    /// Runs `action` on the resumed script state; a panic leaves the
    /// session poisoned.
    fn act(
        &mut self,
        action: impl for<'t> FnOnce(
            &mut FormScripts<'t>,
            &Template<'t>,
        ) -> std::result::Result<ActionOutcome, String>,
    ) -> Result<XfaSessionOutcome> {
        let sources = xfa_sources(&self.loaded).ok_or_else(no_form)?;
        let tpl = Template::new(sources.template);
        let state = self.state.take().ok_or_else(poisoned)?;
        let mut scripts = FormScripts::resume(sources.template, state);
        let result = action(&mut scripts, &tpl);
        self.state = Some(scripts.into_state());
        self.view = None;
        let outcome = result.map_err(WellfriendError::invalid_input)?;
        Ok(XfaSessionOutcome {
            committed: outcome.committed,
            events: outcome
                .fired
                .into_iter()
                .map(|event| XfaFiredEvent {
                    som: event.som,
                    activity: event.activity,
                    scripts: event.scripts,
                })
                .collect(),
            messages: outcome.messages,
            errors: outcome.errors,
            validations: self.validation_results(),
        })
    }

    /// The fields and exclusion groups, in document order.
    pub fn fields(&self) -> Result<Vec<XfaFieldState>> {
        Ok(self
            .state()?
            .session_nodes()
            .into_iter()
            .map(|node| {
                let checked = (node.ui == "checkButton").then(|| {
                    node.value.as_deref() == Some(node.on_value.as_deref().unwrap_or("1"))
                });
                XfaFieldState {
                    som: node.som,
                    name: node.name,
                    kind: node.class.to_string(),
                    ui: node.ui,
                    value_type: node.value_kind,
                    value: node.value,
                    interactive: node.interactive && node.access == "open",
                    presence: node.presence,
                    access: node.access,
                    caption: node.caption,
                    items: node
                        .items
                        .into_iter()
                        .map(|(display, save)| XfaChoiceItem { display, save })
                        .collect(),
                    on_value: node.on_value,
                    off_value: node.off_value,
                    checked,
                }
            })
            .collect())
    }

    /// The raw value of the field (or other node) `som` names.
    pub fn field_value(&mut self, som: &str) -> Result<Option<String>> {
        let state = self.state.as_mut().ok_or_else(poisoned)?;
        state.begin_action();
        state.value_of(som).map_err(WellfriendError::invalid_input)
    }

    /// Edits a field as a user would: `enter`, `change` (scripts see
    /// `xfa.event.prevText` / `newText` and may rewrite `change` or cancel),
    /// `full` at `maxChars`, the commit, `exit`, then the calculations and
    /// validations depending on it. Drop-down lists also fire `preOpen` /
    /// `postOpen` and accept display text for their save value; an
    /// exclusion group takes the on value of the member to select.
    pub fn set_field_value(&mut self, som: &str, value: &str) -> Result<XfaSessionOutcome> {
        self.act(|scripts, tpl| scripts.session_set_value(tpl, som, value))
    }

    /// Clicks a field: buttons fire `click`; check boxes toggle and radio
    /// buttons select (clearing the rest of their exclusion group), firing
    /// `change` and `click` on the group and the button.
    pub fn click(&mut self, som: &str) -> Result<XfaSessionOutcome> {
        self.act(|scripts, _| scripts.session_click(som))
    }

    /// Dispatches one event (`activity`: `click`, `mouseUp`, `preOpen`,
    /// `validate`, a custom name...) to the node `som` names.
    pub fn dispatch_event(&mut self, som: &str, activity: &str) -> Result<XfaSessionOutcome> {
        self.act(|scripts, _| scripts.session_dispatch(som, activity))
    }

    /// Failed validations (`nullTest`, `formatTest`, validate scripts) of
    /// the current form state, in document order.
    pub fn validation_results(&self) -> Vec<XfaValidationResult> {
        self.state
            .as_ref()
            .map(|state| {
                state
                    .report()
                    .validations
                    .into_iter()
                    .map(|record| XfaValidationResult {
                        som: record.som,
                        test: record.test.to_string(),
                        severity: record.severity,
                        message: record.message,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Every `app.alert` / `xfa.host.messageBox` message so far.
    pub fn script_log(&self) -> Vec<String> {
        self.state
            .as_ref()
            .map(ScriptState::session_log)
            .unwrap_or_default()
    }

    /// The first script errors raised so far.
    pub fn script_errors(&self) -> Vec<String> {
        self.state
            .as_ref()
            .map(|state| state.report().errors)
            .unwrap_or_default()
    }

    /// The datasets packet (`<xfa:datasets>`) of the current data DOM.
    pub fn export_data_xml(&self) -> Result<String> {
        Ok(self.state()?.datasets_xml())
    }

    /// The document with the current form state painted.
    fn view(&mut self) -> Result<&ContentEngine> {
        if self.view.is_none() {
            let sources = xfa_sources(&self.loaded).ok_or_else(no_form)?;
            let template = sources.template;
            let tpl = Template::new(template);
            let mut model = build_form_model_with_datasets(
                &tpl,
                template,
                sources.datasets,
                sources.datasets_root,
            )
            .ok_or_else(no_form)?;
            let state = self.state.take().ok_or_else(poisoned)?;
            let mut scripts = FormScripts::resume(template, state);
            scripts.begin_render();
            scripts.apply_to_fresh_model(&tpl, &mut model);
            let painted = paint_form(
                self.engine.document(),
                &sources,
                &tpl,
                model,
                Some(&mut scripts),
            );
            self.state = Some(scripts.into_state());
            self.view = Some(self.engine.with_xfa_render_model(painted));
        }
        self.view.as_ref().ok_or_else(no_form)
    }

    /// Number of XFA pages of the current layout (the PDF's page count for
    /// static forms).
    pub fn page_count(&mut self) -> Result<usize> {
        self.view()?.xfa_page_count()
    }

    /// Renders XFA page `page_number` (1-based) with the session's state:
    /// re-merged and re-laid out, so instance and presence changes show.
    pub fn render_xfa_page(&mut self, page_number: usize, dpi: u32) -> Result<PixelBuffer> {
        self.view()?.render_xfa_page(page_number, dpi)
    }

    /// Same as [`Self::render_xfa_page`].
    pub fn render_page(&mut self, page_number: usize, dpi: u32) -> Result<PixelBuffer> {
        self.render_xfa_page(page_number, dpi)
    }

    /// [`Self::render_xfa_page`] encoded as PNG.
    pub fn render_page_png(&mut self, page_number: usize, dpi: u32) -> Result<Vec<u8>> {
        let buffer = self.render_xfa_page(page_number, dpi)?;
        ImageEncoder::encode_png_fast(&buffer.into_raw_image())
    }

    /// The current layout as a standalone PDF (one page per XFA page, the
    /// form painted as page content).
    pub fn layout_pdf(&mut self) -> Result<Vec<u8>> {
        self.view()?.xfa_layout_pdf()
    }

    /// Fires `preSave`, writes the data DOM into the PDF's `datasets`
    /// packet as an incremental update (adding the packet when missing),
    /// then fires `postSave`. The session itself keeps working on its
    /// state; the returned bytes are the saved document.
    pub fn save_to_pdf(&mut self) -> Result<Vec<u8>> {
        self.broadcast("preSave")?;
        let xml = self.export_data_xml()?;
        let bytes = write_datasets(&self.engine, &xml)?;
        self.broadcast("postSave")?;
        Ok(bytes)
    }

    fn broadcast(&mut self, activity: &str) -> Result<()> {
        self.act(|scripts, _| Ok(scripts.session_broadcast(activity)))
            .map(|_| ())
    }

    /// The JSON command transport used by the C ABI, Python and WASM
    /// bindings. A command is an object with an `op`:
    ///
    /// * `{"op":"fields"}` → `{"fields":[XfaFieldState...]}`
    /// * `{"op":"value","som":S}` → `{"value":V|null}`
    /// * `{"op":"set_value","som":S,"value":V}`, `{"op":"click","som":S}`,
    ///   `{"op":"dispatch","som":S,"activity":A}` → [`XfaSessionOutcome`]
    /// * `{"op":"validations"}` → `{"validations":[...]}`
    /// * `{"op":"script_log"}` → `{"messages":[...],"errors":[...]}`
    /// * `{"op":"export_data_xml"}` → `{"xml":"<xfa:datasets...>"}`
    /// * `{"op":"page_count"}` → `{"page_count":N}`
    /// * `{"op":"form_type"}` → `{"form_type":"foreground"|"full"}`
    pub fn command_json(&mut self, command: &str) -> Result<String> {
        let command: serde_json::Value = serde_json::from_str(command)
            .map_err(|err| WellfriendError::invalid_input(format!("XFA session command: {err}")))?;
        let text = |key: &str| -> Result<String> {
            command
                .get(key)
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| {
                    WellfriendError::invalid_input(format!(
                        "XFA session command needs a string \"{key}\""
                    ))
                })
        };
        let op = text("op")?;
        let json = |value: XfaSessionOutcome| {
            serde_json::to_value(value)
                .map_err(|err| WellfriendError::invalid_input(format!("XFA session output: {err}")))
        };
        let value = match op.as_str() {
            "fields" => serde_json::json!({ "fields": self.fields()? }),
            "value" => serde_json::json!({ "value": self.field_value(&text("som")?)? }),
            "set_value" => {
                let value = match command.get("value") {
                    Some(serde_json::Value::String(text)) => text.clone(),
                    Some(serde_json::Value::Null) | None => String::new(),
                    Some(other) => other.to_string(),
                };
                json(self.set_field_value(&text("som")?, &value)?)?
            }
            "click" => json(self.click(&text("som")?)?)?,
            "dispatch" => json(self.dispatch_event(&text("som")?, &text("activity")?)?)?,
            "validations" => serde_json::json!({ "validations": self.validation_results() }),
            "script_log" => serde_json::json!({
                "messages": self.script_log(),
                "errors": self.script_errors(),
            }),
            "export_data_xml" => serde_json::json!({ "xml": self.export_data_xml()? }),
            "page_count" => serde_json::json!({ "page_count": self.page_count()? }),
            "form_type" => serde_json::json!({ "form_type": self.form_type() }),
            other => {
                return Err(WellfriendError::invalid_input(format!(
                    "unknown XFA session op {other:?}"
                )))
            }
        };
        Ok(value.to_string())
    }
}

/// Zlib-compressed stream object holding `bytes`.
fn flate_stream(bytes: &[u8]) -> Result<PdfObject> {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    std::io::Write::write_all(&mut encoder, bytes)
        .map_err(|err| WellfriendError::MalformedPdf(err.to_string()))?;
    let raw = encoder
        .finish()
        .map_err(|err| WellfriendError::MalformedPdf(err.to_string()))?;
    let mut dict = PdfDictionary::empty();
    dict.insert("Filter", PdfObject::Name("FlateDecode".into()));
    Ok(PdfObject::Stream { dict, raw })
}

fn is_packet_name(object: &PdfObject, name: &str) -> bool {
    matches!(object, PdfObject::String(bytes) if crate::info::decode_pdf_text_string(bytes) == name)
}

/// Writes `datasets` (a `<xfa:datasets>` element) into the document's XFA
/// entry: the `datasets` stream of a packet array is replaced (or a new one
/// inserted before the last packet, as PDFium), a single XDP stream gets
/// its `datasets` element spliced. Incremental update.
fn write_datasets(engine: &ContentEngine, datasets: &str) -> Result<Vec<u8>> {
    let document = engine.document();
    let reader = document.reader();
    if reader.is_encrypted() {
        return Err(WellfriendError::UnsupportedFeature(
            "saving XFA data into an encrypted PDF is not supported".into(),
        ));
    }
    let catalog = document.get_catalog()?;
    let acroform_entry = catalog.get("AcroForm").cloned().ok_or_else(no_form)?;
    let mut acroform = reader
        .resolve(acroform_entry.clone())?
        .as_dict()
        .cloned()
        .ok_or_else(no_form)?;
    let xfa_entry = acroform.get("XFA").cloned().ok_or_else(no_form)?;
    let mut changes: Vec<IncrementalObject> = Vec::new();
    let mut next = reader
        .object_ids()
        .into_iter()
        .map(|id| id.0)
        .max()
        .unwrap_or(0);
    match reader.resolve(xfa_entry.clone())? {
        PdfObject::Array(mut items) => {
            let index = (0..items.len().saturating_sub(1))
                .find(|&i| is_packet_name(&items[i], "datasets"))
                .map(|i| i + 1);
            let stream = flate_stream(datasets.as_bytes())?;
            if let Some((number, generation)) = index.and_then(|i| items[i].as_reference()) {
                changes.push(IncrementalObject {
                    number,
                    generation,
                    object: stream,
                });
            } else {
                next += 1;
                changes.push(IncrementalObject {
                    number: next,
                    generation: 0,
                    object: stream,
                });
                let reference = PdfObject::Reference {
                    number: next,
                    generation: 0,
                };
                match index {
                    Some(i) => items[i] = reference,
                    None => {
                        let at = items.len().saturating_sub(2);
                        items.insert(at, PdfObject::String(b"datasets".to_vec()));
                        items.insert(at + 1, reference);
                    }
                }
                let array = PdfObject::Array(items);
                if let Some((number, generation)) = xfa_entry.as_reference() {
                    changes.push(IncrementalObject {
                        number,
                        generation,
                        object: array,
                    });
                } else {
                    acroform.insert("XFA", array);
                    push_acroform(reader, &catalog, &acroform_entry, acroform, &mut changes)?;
                }
            }
        }
        stream @ PdfObject::Stream { .. } => {
            let (number, generation) = xfa_entry.as_reference().ok_or_else(no_form)?;
            let bytes = crate::filters::decode_stream_with_limits(
                &stream,
                reader,
                &crate::filters::DecodeLimits::default(),
            )?;
            let spliced = splice_datasets(&bytes, datasets)?;
            changes.push(IncrementalObject {
                number,
                generation,
                object: flate_stream(&spliced)?,
            });
        }
        _ => return Err(no_form()),
    }
    write_incremental_update(reader, changes)
}

/// Writes a changed AcroForm dictionary: in place when it is an indirect
/// object, else through the catalog.
fn push_acroform(
    reader: &crate::reader::PdfReader,
    catalog: &PdfDictionary,
    entry: &PdfObject,
    acroform: PdfDictionary,
    changes: &mut Vec<IncrementalObject>,
) -> Result<()> {
    if let Some((number, generation)) = entry.as_reference() {
        changes.push(IncrementalObject {
            number,
            generation,
            object: PdfObject::Dictionary(acroform),
        });
        return Ok(());
    }
    let (number, generation) = reader
        .trailer()
        .get("Root")
        .and_then(PdfObject::as_reference)
        .ok_or_else(|| WellfriendError::MalformedPdf("catalog is not an indirect object".into()))?;
    let mut catalog = catalog.clone();
    catalog.insert("AcroForm", PdfObject::Dictionary(acroform));
    changes.push(IncrementalObject {
        number,
        generation,
        object: PdfObject::Dictionary(catalog),
    });
    Ok(())
}

/// Replaces the `datasets` element of a whole-XDP stream (or inserts one
/// before the closing root tag).
fn splice_datasets(xdp: &[u8], datasets: &str) -> Result<Vec<u8>> {
    let parsed = parse_xml(xdp, &XfaLimits::default())?;
    let root = &parsed.root;
    let mut out = Vec::with_capacity(xdp.len() + datasets.len());
    if let Some(existing) = root.children.iter().find(|c| c.local_name == "datasets") {
        out.extend_from_slice(&xdp[..existing.start_offset]);
        out.extend_from_slice(datasets.as_bytes());
        out.extend_from_slice(&xdp[existing.end_offset..]);
        return Ok(out);
    }
    let close = format!("</{}>", root.name);
    let at = root
        .end_offset
        .checked_sub(close.len())
        .filter(|&at| xdp.get(at..root.end_offset) == Some(close.as_bytes()))
        .ok_or_else(|| WellfriendError::MalformedPdf("XDP root element has no end tag".into()))?;
    out.extend_from_slice(&xdp[..at]);
    out.extend_from_slice(datasets.as_bytes());
    out.extend_from_slice(&xdp[at..]);
    Ok(out)
}
