//! Retained, caller-owned editing for a dedicated browser worker. No host I/O.
use serde::{de::DeserializeOwned, Serialize};
use wasm_bindgen::prelude::*;
use wellfriendpdf_engine::linked_stories::{
    LinkedStoryRequest, LinkedStorySession, StoryPreviewReceipt,
};
use wellfriendpdf_engine::story_session_protocol as protocol;
use wellfriendpdf_engine::{CancelToken, WellfriendError};

fn err(error: WellfriendError) -> JsValue {
    JsValue::from_str(&error.to_string())
}
fn encode(value: &impl Serialize) -> Result<String, JsValue> {
    serde_json::to_string(value).map_err(|e| JsValue::from_str(&e.to_string()))
}
fn decode<T: DeserializeOwned>(json: &str) -> Result<T, JsValue> {
    if json.len() > 32 * 1024 * 1024 {
        return Err(JsValue::from_str(
            "story JSON exceeds 32 MiB binding budget",
        ));
    }
    serde_json::from_str(json).map_err(|e| JsValue::from_str(&e.to_string()))
}

#[wasm_bindgen]
pub struct StoryEditSession {
    inner: Option<LinkedStorySession>,
}

impl StoryEditSession {
    fn dispatch(&mut self, command: protocol::StorySessionCommand) -> Result<String, JsValue> {
        let bytes =
            protocol::execute(self.get_mut()?, command, &CancelToken::none()).map_err(err)?;
        String::from_utf8(bytes).map_err(|e| JsValue::from_str(&e.to_string()))
    }
    fn get(&self) -> Result<&LinkedStorySession, JsValue> {
        self.inner
            .as_ref()
            .ok_or_else(|| JsValue::from_str("story session is closed"))
    }
    fn get_mut(&mut self) -> Result<&mut LinkedStorySession, JsValue> {
        self.inner
            .as_mut()
            .ok_or_else(|| JsValue::from_str("story session is closed"))
    }
}

#[wasm_bindgen]
impl StoryEditSession {
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: &[u8]) -> Result<StoryEditSession, JsValue> {
        #[cfg(feature = "panic-hook")]
        console_error_panic_hook::set_once();
        Ok(Self {
            inner: Some(protocol::open(bytes, &CancelToken::none()).map_err(err)?),
        })
    }
    /// Unlock a Standard-handler encrypted PDF with its permissions/owner
    /// password and retain the canonical
    /// unencrypted working revision. Status JSON reports this transition and
    /// the password bytes are not stored by the session.
    #[wasm_bindgen(js_name = openWithPassword)]
    pub fn open_with_password(bytes: &[u8], password: &[u8]) -> Result<StoryEditSession, JsValue> {
        #[cfg(feature = "panic-hook")]
        console_error_panic_hook::set_once();
        Ok(Self {
            inner: Some(
                protocol::open_with_password(bytes, password, &CancelToken::none()).map_err(err)?,
            ),
        })
    }
    pub fn close(&mut self) {
        self.inner = None;
    }
    /// The same bounded command envelope exposed by C/Java/.NET. Native and
    /// browser hosts share revision/receipt/merge and cancellation semantics.
    #[wasm_bindgen(js_name = commandJson)]
    pub fn command_json(&mut self, command_json: &str) -> Result<String, JsValue> {
        let bytes = protocol::execute_json(
            self.get_mut()?,
            command_json.as_bytes(),
            &CancelToken::none(),
        )
        .map_err(err)?;
        String::from_utf8(bytes).map_err(|e| JsValue::from_str(&e.to_string()))
    }
    pub fn bytes(&self) -> Result<Vec<u8>, JsValue> {
        Ok(self.get()?.bytes().to_vec())
    }
    #[wasm_bindgen(js_name = revisionSha256)]
    pub fn revision_sha256(&self) -> Result<String, JsValue> {
        Ok(self.get()?.revision_sha256())
    }
    #[wasm_bindgen(js_name = savedStoriesJson)]
    pub fn saved_stories_json(&self) -> Result<String, JsValue> {
        encode(&self.get()?.saved_stories().map_err(err)?)
    }
    #[wasm_bindgen(js_name = pagesJson)]
    pub fn pages_json(&self) -> Result<String, JsValue> {
        let pages = self.get()?.document().document().get_pages().map_err(err)?;
        encode(
            &pages
                .iter()
                .map(|p| {
                    serde_json::json!({"page":p.page_number,"crop_box":p.crop_box,
            "media_box":p.media_box,"rotate":p.rotate,"user_unit":p.user_unit})
                })
                .collect::<Vec<_>>(),
        )
    }
    #[wasm_bindgen(js_name = sourceModelJson)]
    pub fn source_model_json(&self, page: usize) -> Result<String, JsValue> {
        encode(
            &wellfriendpdf_engine::advanced_editing::analyze_multi_run_text_range(
                self.get()?.bytes(),
                page,
            )
            .map_err(err)?,
        )
    }
    #[wasm_bindgen(js_name = annotationSourcesJson)]
    pub fn annotation_sources_json(&self) -> Result<String, JsValue> {
        encode(
            &wellfriendpdf_engine::story_anchors::annotation_anchor_sources(self.get()?.bytes())
                .map_err(err)?,
        )
    }
    #[wasm_bindgen(js_name = imageSourcesJson)]
    pub fn image_sources_json(&self, page: usize) -> Result<String, JsValue> {
        encode(
            &wellfriendpdf_engine::universal_editing::universal_image_occurrences_v2(
                self.get()?.bytes(),
                &[page],
            )
            .map_err(err)?,
        )
    }
    #[wasm_bindgen(js_name = tagSourcesJson)]
    pub fn tag_sources_json(&self) -> Result<String, JsValue> {
        encode(
            &wellfriendpdf_engine::tagged_structure::story::sources(self.get()?.bytes())
                .map_err(err)?,
        )
    }
    #[wasm_bindgen(js_name = pageGeometryJson)]
    pub fn page_geometry_json(&self, page: usize, dpi: u32) -> Result<String, JsValue> {
        if dpi == 0 || dpi > 300 {
            return Err(JsValue::from_str("preview DPI must be in 1..=300"));
        }
        let viewport = self
            .get()?
            .document()
            .page_viewport(page, dpi)
            .map_err(err)?;
        let m = viewport.to_transform();
        encode(
            &serde_json::json!({"page":page,"width":viewport.width_px,"height":viewport.height_px,
            "pdf_to_device":[m.a,m.b,m.c,m.d,m.e,m.f]}),
        )
    }
    /// Geometry/shaping preview only; no claim to reproduce native glyph pixels
    /// using a browser DOM font. Publication requires the returned receipt.
    #[wasm_bindgen(js_name = previewJson)]
    pub fn preview_json(&mut self, request_json: &str) -> Result<String, JsValue> {
        let request: LinkedStoryRequest = decode(request_json)?;
        self.dispatch(protocol::StorySessionCommand::Preview { request })
    }
    /// Pure draft recalculation; a subsequent preview/receipt is still required.
    #[wasm_bindgen(js_name = synchronizeTableValuesJson)]
    pub fn synchronize_table_values_json(&mut self, request_json: &str) -> Result<String, JsValue> {
        let request: LinkedStoryRequest = decode(request_json)?;
        self.dispatch(protocol::StorySessionCommand::SynchronizeTableValues { request })
    }
    #[wasm_bindgen(js_name = checkpointJson)]
    pub fn checkpoint_json(
        &mut self,
        request_json: &str,
        receipt_json: &str,
    ) -> Result<String, JsValue> {
        let request: LinkedStoryRequest = decode(request_json)?;
        let receipt: StoryPreviewReceipt = decode(receipt_json)?;
        self.dispatch(protocol::StorySessionCommand::Checkpoint { request, receipt })
    }
    pub fn undo(&mut self) -> Result<bool, JsValue> {
        self.get_mut()?.undo().map_err(err)
    }
    pub fn redo(&mut self) -> Result<bool, JsValue> {
        self.get_mut()?.redo().map_err(err)
    }

    #[wasm_bindgen(js_name = mergeTextJson)]
    pub fn merge_text_json(&mut self, request_json: &str) -> Result<String, JsValue> {
        let request: wellfriendpdf_engine::story_merge::StoryMergeRequest = decode(request_json)?;
        self.dispatch(protocol::StorySessionCommand::MergeText { request })
    }
    #[wasm_bindgen(js_name = mergeStructureJson)]
    pub fn merge_structure_json(&mut self, request_json: &str) -> Result<String, JsValue> {
        let request: wellfriendpdf_engine::story_structure_merge::StoryStructureMergeRequest =
            decode(request_json)?;
        self.dispatch(protocol::StorySessionCommand::MergeStructure { request })
    }
    /// Rendering executes only when called by the host. This addition was not
    /// executed during implementation. It reuses the canonical native renderer.
    #[wasm_bindgen(js_name = renderPagePng)]
    pub fn render_page_png(&self, page: usize, dpi: u32) -> Result<Vec<u8>, JsValue> {
        protocol::render_page_png(self.get()?, page, dpi, &CancelToken::none()).map_err(err)
    }
}
