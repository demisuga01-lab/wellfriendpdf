//! Stateless native-image transactions for caller-owned bytes. Invoke from a
//! worker; this facade does not alter a retained StoryEditSession implicitly.
use serde::Serialize;
use wasm_bindgen::prelude::*;
use wellfriendpdf_engine::image_fragments::{self, ImageFragmentMove};

fn check_input(input: &[u8]) -> Result<(), JsValue> {
    if input.len() > 256 * 1024 * 1024 {
        return Err(JsValue::from_str("image fragment input exceeds 256 MiB"));
    }
    Ok(())
}
fn error(e: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&e.to_string())
}
fn encode(value: &impl Serialize) -> Result<String, JsValue> {
    serde_json::to_string(value).map_err(error)
}
fn request(json: &str) -> Result<ImageFragmentMove, JsValue> {
    if json.len() > 8 * 1024 * 1024 {
        return Err(JsValue::from_str("image fragment request exceeds 8 MiB"));
    }
    serde_json::from_str(json).map_err(error)
}

#[wasm_bindgen]
pub struct ImageFragmentOutput {
    bytes: Vec<u8>,
    report: String,
}
#[wasm_bindgen]
impl ImageFragmentOutput {
    pub fn bytes(&self) -> Vec<u8> {
        self.bytes.clone()
    }
    #[wasm_bindgen(js_name = reportJson)]
    pub fn report_json(&self) -> String {
        self.report.clone()
    }
}

#[wasm_bindgen(js_name = imageFragmentBindingsJson)]
pub fn image_fragment_bindings_json(input: &[u8]) -> Result<String, JsValue> {
    check_input(input)?;
    encode(&image_fragments::image_fragment_bindings(input).map_err(error)?)
}

/// Full page-logical source model, including visible spans so the client can
/// distinguish duplicate wording. Only explicit Tr=3 operands may be captured.
#[wasm_bindgen(js_name = imageOcrSourcesJson)]
pub fn image_ocr_sources_json(input: &[u8], page: usize) -> Result<String, JsValue> {
    check_input(input)?;
    encode(
        &wellfriendpdf_engine::advanced_editing::analyze_multi_run_text_range(input, page)
            .map_err(error)?,
    )
}

#[wasm_bindgen(js_name = previewImageFragmentMoveJson)]
pub fn preview_image_fragment_move_json(
    input: &[u8],
    request_json: &str,
) -> Result<String, JsValue> {
    check_input(input)?;
    encode(
        &image_fragments::preview_image_fragment_move(input, &request(request_json)?)
            .map_err(error)?,
    )
}

#[wasm_bindgen(js_name = applyImageFragmentMove)]
pub fn apply_image_fragment_move(
    input: &[u8],
    request_json: &str,
    approved_plan_sha256: &str,
) -> Result<ImageFragmentOutput, JsValue> {
    check_input(input)?;
    if approved_plan_sha256.len() != 64 {
        return Err(JsValue::from_str("invalid image fragment preview receipt"));
    }
    let (bytes, report) = image_fragments::apply_image_fragment_move(
        input,
        &request(request_json)?,
        approved_plan_sha256,
    )
    .map_err(error)?;
    Ok(ImageFragmentOutput {
        bytes,
        report: encode(&report)?,
    })
}
