//! The browser PDF editor's engine session: render pages, map text and
//! objects for selection, and preview operations with the same
//! `wellfriendpdf-editor` code the server replays on save.
use js_sys::{Object, Reflect, Uint8ClampedArray};
use wasm_bindgen::prelude::*;
use wellfriendpdf_editor::{
    apply_operation, ops::validate_operation, EditorOperation, MemoryFonts,
};
use wellfriendpdf_engine::{
    CancelToken, ContentEngine, PageTextLayout, RenderDocumentCache, RenderMode,
};

fn js_err(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}

fn editor_err(error: wellfriendpdf_editor::EditorError) -> JsValue {
    JsValue::from_str(&serde_json::to_string(&error).unwrap_or_else(|_| error.to_string()))
}

fn lopdf_fonts(bytes: &[u8], page: u32) -> Vec<wellfriendpdf_editor::SourceFont> {
    wellfriendpdf_editor::overlay::load(bytes)
        .map(|document| wellfriendpdf_editor::fonts::page_fonts(&document, page))
        .unwrap_or_default()
}

#[wasm_bindgen]
pub struct PdfEditorSession {
    bytes: Vec<u8>,
    engine: ContentEngine,
    cache: RenderDocumentCache,
    fonts: MemoryFonts,
}

#[wasm_bindgen]
impl PdfEditorSession {
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: &[u8]) -> Result<PdfEditorSession, JsValue> {
        console_error_panic_hook::set_once();
        wellfriendpdf_editor::apply::host_independent();
        let engine = ContentEngine::open_bytes(bytes.to_vec()).map_err(js_err)?;
        Ok(Self {
            bytes: bytes.to_vec(),
            engine,
            cache: RenderDocumentCache::new(),
            fonts: MemoryFonts::new(),
        })
    }

    #[wasm_bindgen(js_name = pageCount)]
    pub fn page_count(&self) -> Result<usize, JsValue> {
        self.engine.page_count().map_err(js_err)
    }

    /// Every page's displayed size in points (rotation and `/UserUnit`
    /// applied): `[[width, height], ...]`. Cheap: no content is parsed.
    #[wasm_bindgen(js_name = pageSizes)]
    pub fn page_sizes(&self) -> Result<String, JsValue> {
        let count = self.engine.page_count().map_err(js_err)?;
        let mut sizes = Vec::with_capacity(count);
        for page in 1..=count {
            let viewport = self.engine.page_viewport(page, 720).map_err(js_err)?;
            sizes.push([
                f64::from(viewport.width_px) / 10.0,
                f64::from(viewport.height_px) / 10.0,
            ]);
        }
        serde_json::to_string(&sizes).map_err(js_err)
    }

    /// `{width, height, transform72}`: the displayed size in points and the
    /// user-space to 72-dpi pixel matrix `[a, b, c, d, e, f]` (crop box,
    /// rotation and `/UserUnit` applied). Cheap: no content is parsed.
    #[wasm_bindgen(js_name = pageInfo)]
    pub fn page_info(&self, page: usize) -> Result<String, JsValue> {
        let viewport = self.engine.page_viewport(page, 72).map_err(js_err)?;
        serde_json::to_string(&serde_json::json!({
            "width": viewport.width_px,
            "height": viewport.height_px,
            "transform72": viewport.to_transform().to_array(),
        }))
        .map_err(js_err)
    }

    /// Render a page at `dpi`: `{width, height, data}` with RGBA pixels.
    #[wasm_bindgen(js_name = renderPage)]
    pub fn render_page(&mut self, page: usize, dpi: u32) -> Result<JsValue, JsValue> {
        let buffer = self
            .engine
            .render_page_cancellable_with_mode_and_cache(
                page,
                dpi,
                &CancelToken::none(),
                RenderMode::Compat,
                &mut self.cache,
            )
            .map_err(js_err)?;
        let raw = buffer.into_raw_image();
        let pixels = raw.width as usize * raw.height as usize;
        let mut rgba = Vec::with_capacity(pixels * 4);
        match raw.channels {
            4 => rgba = raw.pixels,
            3 => {
                for chunk in raw.pixels.as_chunks::<3>().0 {
                    rgba.extend_from_slice(&[chunk[0], chunk[1], chunk[2], 255]);
                }
            }
            1 => {
                for g in raw.pixels {
                    rgba.extend_from_slice(&[g, g, g, 255]);
                }
            }
            other => return Err(js_err(format!("unexpected render channel count {other}"))),
        }
        let result = Object::new();
        Reflect::set(&result, &"width".into(), &raw.width.into())?;
        Reflect::set(&result, &"height".into(), &raw.height.into())?;
        Reflect::set(
            &result,
            &"data".into(),
            &Uint8ClampedArray::from(rgba.as_slice()).into(),
        )?;
        Ok(result.into())
    }

    /// Editable text layout of a page (user space): text, glyph quads and
    /// boxes with font resources and sizes, lines, paragraphs.
    #[wasm_bindgen(js_name = textLayout)]
    pub fn text_layout(&self, page: usize) -> Result<String, JsValue> {
        let layout = PageTextLayout::build(&self.engine, page).map_err(js_err)?;
        let fonts = lopdf_fonts(&self.bytes, page as u32);
        let mut value = serde_json::to_value(&layout).map_err(js_err)?;
        value["transform72"] = serde_json::json!(layout.device_transform(72));
        value["fonts"] = serde_json::to_value(fonts).map_err(js_err)?;
        serde_json::to_string(&value).map_err(js_err)
    }

    /// Selectable objects of a page: image placements, vector paths and the
    /// page fonts with their library matches.
    #[wasm_bindgen(js_name = pageObjects)]
    pub fn page_objects(&self, page: u32) -> Result<String, JsValue> {
        let objects = wellfriendpdf_editor::page_objects(&self.bytes, page).map_err(editor_err)?;
        serde_json::to_string(&objects).map_err(js_err)
    }

    /// Make a library font available to added text and substitutions.
    #[wasm_bindgen(js_name = registerFont)]
    pub fn register_font(&mut self, id: &str, bytes: &[u8]) -> bool {
        self.fonts.insert(id, bytes.to_vec())
    }

    #[wasm_bindgen(js_name = hasFont)]
    pub fn has_font(&self, id: &str) -> bool {
        self.fonts.contains(id)
    }

    /// Validate and apply one operation (JSON) to the current bytes. Returns
    /// the operation report JSON; on refusal throws the error JSON and the
    /// document is unchanged.
    pub fn apply(
        &mut self,
        operation_json: &str,
        allow_signature_invalidation: bool,
    ) -> Result<String, JsValue> {
        let operation: EditorOperation = serde_json::from_str(operation_json).map_err(|e| {
            editor_err(wellfriendpdf_editor::EditorError::new(
                "EDITOR_OPERATION_INVALID",
                e.to_string(),
            ))
        })?;
        let mut image_bytes = 0;
        validate_operation(&operation, &mut image_bytes).map_err(editor_err)?;
        let (bytes, report) = apply_operation(
            &self.bytes,
            &operation,
            &self.fonts,
            allow_signature_invalidation,
        )
        .map_err(editor_err)?;
        self.set_bytes(&bytes)?;
        serde_json::to_string(&report).map_err(js_err)
    }

    /// Replace the current document (undo/redo restores a snapshot).
    #[wasm_bindgen(js_name = setBytes)]
    pub fn set_bytes(&mut self, bytes: &[u8]) -> Result<(), JsValue> {
        let engine = ContentEngine::open_bytes(bytes.to_vec()).map_err(js_err)?;
        self.engine = engine;
        self.cache = RenderDocumentCache::new();
        self.bytes = bytes.to_vec();
        Ok(())
    }

    /// A new PDF with the listed pages (JSON array of 1-based numbers).
    #[wasm_bindgen(js_name = extractPages)]
    pub fn extract_pages(&self, pages_json: &str) -> Result<Vec<u8>, JsValue> {
        let pages: Vec<u32> = serde_json::from_str(pages_json).map_err(js_err)?;
        wellfriendpdf_editor::pages::extract_pages(&self.bytes, &pages).map_err(editor_err)
    }

    pub fn bytes(&self) -> Vec<u8> {
        self.bytes.clone()
    }

    #[wasm_bindgen(js_name = byteLength)]
    pub fn byte_length(&self) -> usize {
        self.bytes.len()
    }
}

/// The window of a blank page inserted into a large (windowed) document,
/// the same bytes the server builds when it saves the insertion.
#[wasm_bindgen(js_name = blankWindow)]
pub fn blank_window(width: f64, height: f64) -> Result<Vec<u8>, JsValue> {
    wellfriendpdf_editor::windows::blank_window(width, height).map_err(editor_err)
}

/// The windows of pages of another PDF inserted into a large document (all
/// pages when `pages_json` is `[]`), one per page, as the server builds them.
#[wasm_bindgen(js_name = insertedWindows)]
pub fn inserted_windows(pdf: &[u8], pages_json: &str) -> Result<js_sys::Array, JsValue> {
    wellfriendpdf_editor::apply::host_independent();
    let pages: Vec<u32> = serde_json::from_str(pages_json).map_err(js_err)?;
    let windows = wellfriendpdf_editor::windows::inserted_windows(pdf, &pages).map_err(editor_err)?;
    let out = js_sys::Array::new();
    for window in windows {
        out.push(&js_sys::Uint8Array::from(window.as_slice()));
    }
    Ok(out)
}
