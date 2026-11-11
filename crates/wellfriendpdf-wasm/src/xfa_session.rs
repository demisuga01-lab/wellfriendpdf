//! Interactive XFA form sessions for browsers: JSON commands in/out, bytes
//! for rendered pages and saved PDFs. No host I/O.
use wasm_bindgen::prelude::*;
use wellfriendpdf_engine::{ContentEngine, WellfriendError, XfaFormSession, XfaSessionOptions};

fn err(error: WellfriendError) -> JsValue {
    JsValue::from_str(&error.to_string())
}

#[wasm_bindgen]
pub struct XfaSession {
    inner: Option<XfaFormSession>,
}

impl XfaSession {
    fn get_mut(&mut self) -> Result<&mut XfaFormSession, JsValue> {
        self.inner
            .as_mut()
            .ok_or_else(|| JsValue::from_str("XFA session is closed"))
    }
}

#[wasm_bindgen]
impl XfaSession {
    /// Opens a session over PDF bytes. `options_json` is optional
    /// `{"run_scripts":bool}`.
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: &[u8], options_json: Option<String>) -> Result<XfaSession, JsValue> {
        #[cfg(feature = "panic-hook")]
        console_error_panic_hook::set_once();
        let options = match options_json {
            Some(json) if !json.trim().is_empty() => {
                serde_json::from_str::<XfaSessionOptions>(&json)
                    .map_err(|e| JsValue::from_str(&e.to_string()))?
            }
            _ => XfaSessionOptions::default(),
        };
        let engine = ContentEngine::open_bytes(bytes.to_vec()).map_err(err)?;
        Ok(Self {
            inner: Some(engine.xfa_session_with_options(&options).map_err(err)?),
        })
    }

    pub fn close(&mut self) {
        self.inner = None;
    }

    /// Runs a JSON command (`fields`, `value`, `set_value`, `type_text`, `click`,
    /// `dispatch`, `validations`, `script_log`, `export_data_xml`,
    /// `page_count`, `form_type`) and returns its JSON result.
    #[wasm_bindgen(js_name = commandJson)]
    pub fn command_json(&mut self, command_json: &str) -> Result<String, JsValue> {
        if command_json.len() > 32 * 1024 * 1024 {
            return Err(JsValue::from_str("XFA command exceeds 32 MiB"));
        }
        self.get_mut()?.command_json(command_json).map_err(err)
    }

    /// PNG of XFA page `page` (1-based) with the session's state.
    #[wasm_bindgen(js_name = renderPagePng)]
    pub fn render_page_png(&mut self, page: usize, dpi: u32) -> Result<Vec<u8>, JsValue> {
        if dpi == 0 || dpi > 600 {
            return Err(JsValue::from_str("dpi must be 1..=600"));
        }
        self.get_mut()?.render_page_png(page, dpi).map_err(err)
    }

    /// The PDF with the session's data saved into its datasets packet.
    #[wasm_bindgen(js_name = savePdf)]
    pub fn save_pdf(&mut self) -> Result<Vec<u8>, JsValue> {
        self.get_mut()?.save_to_pdf().map_err(err)
    }

    /// The current XFA layout as a standalone PDF.
    #[wasm_bindgen(js_name = layoutPdf)]
    pub fn layout_pdf(&mut self) -> Result<Vec<u8>, JsValue> {
        self.get_mut()?.layout_pdf().map_err(err)
    }
}
