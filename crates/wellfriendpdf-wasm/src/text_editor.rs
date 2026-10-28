//! Click-to-edit text editing for browser UIs. The engine supplies text,
//! geometry and edits; the page decides how carets, selections and input
//! look. Coordinates are rendered pixels at the caller's DPI unless a method
//! says otherwise; offsets are scalar positions in the page's layout text.
use serde::Serialize;
use wasm_bindgen::prelude::*;
use wellfriendpdf_engine::{CaretMove, PageTextLayout, TextEditSession, WellfriendError};

fn err(error: WellfriendError) -> JsValue {
    JsValue::from_str(&error.to_string())
}
fn encode(value: &impl Serialize) -> Result<String, JsValue> {
    serde_json::to_string(value).map_err(|e| JsValue::from_str(&e.to_string()))
}

#[derive(Serialize)]
struct DeviceCaret {
    offset: usize,
    line: usize,
    top: [f64; 2],
    bottom: [f64; 2],
}

#[wasm_bindgen]
pub struct TextEditor {
    session: TextEditSession,
}

impl TextEditor {
    fn layout(&mut self, page: usize) -> Result<&PageTextLayout, JsValue> {
        self.session.layout(page).map_err(err)
    }
}

#[wasm_bindgen]
impl TextEditor {
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: &[u8]) -> Result<TextEditor, JsValue> {
        Ok(Self {
            session: TextEditSession::open(bytes.to_vec()).map_err(err)?,
        })
    }

    /// Page layout JSON (PDF user space): `text`, `glyphs` with quads,
    /// `lines`, `paragraphs`, `page_box`, `rotation`.
    #[wasm_bindgen(js_name = layoutJson)]
    pub fn layout_json(&mut self, page: usize) -> Result<String, JsValue> {
        encode(self.layout(page)?)
    }

    /// Page-to-pixel transform `[a, b, c, d, e, f]` of a render at `dpi`.
    #[wasm_bindgen(js_name = deviceTransform)]
    pub fn device_transform(&mut self, page: usize, dpi: u32) -> Result<Vec<f64>, JsValue> {
        Ok(self.layout(page)?.device_transform(dpi).to_vec())
    }

    /// Caret offset for a click at pixel (`x`, `y`), or -1 on a page
    /// without text.
    #[wasm_bindgen(js_name = hitTest)]
    pub fn hit_test(&mut self, page: usize, x: f64, y: f64, dpi: u32) -> Result<i64, JsValue> {
        Ok(self
            .layout(page)?
            .hit_test_device([x, y], dpi)
            .map_or(-1, |offset| offset as i64))
    }

    /// Caret JSON in pixels: `{offset, line, top: [x, y], bottom: [x, y]}`.
    #[wasm_bindgen(js_name = caretJson)]
    pub fn caret_json(&mut self, page: usize, offset: usize, dpi: u32) -> Result<String, JsValue> {
        let layout = self.layout(page)?;
        let caret = layout.caret(offset).map(|caret| DeviceCaret {
            offset: caret.offset,
            line: caret.line,
            top: layout.to_device(caret.top, dpi),
            bottom: layout.to_device(caret.bottom, dpi),
        });
        encode(&caret)
    }

    /// Selection highlight quads in pixels, one per line: `[[[x, y] x4], ...]`.
    #[wasm_bindgen(js_name = selectionJson)]
    pub fn selection_json(
        &mut self,
        page: usize,
        start: usize,
        end: usize,
        dpi: u32,
    ) -> Result<String, JsValue> {
        let layout = self.layout(page)?;
        let quads: Vec<[[f64; 2]; 4]> = layout
            .selection_quads([start, end])
            .into_iter()
            .map(|quad| quad.map(|point| layout.to_device(point, dpi)))
            .collect();
        encode(&quads)
    }

    /// The text of `[start, end)`.
    #[wasm_bindgen(js_name = textIn)]
    pub fn text_in(&mut self, page: usize, start: usize, end: usize) -> Result<String, JsValue> {
        Ok(self.layout(page)?.text_in([start, end]))
    }

    /// `[start, end]` of the word, line or paragraph (`unit`) around `offset`.
    pub fn range(&mut self, page: usize, offset: usize, unit: &str) -> Result<Vec<usize>, JsValue> {
        let layout = self.layout(page)?;
        let range = match unit {
            "word" => layout.word_range(offset),
            "line" => layout.line_range(offset),
            "paragraph" => layout.paragraph_range(offset),
            other => {
                return Err(JsValue::from_str(&format!(
                    "unknown range unit {other}; use word, line or paragraph"
                )))
            }
        };
        Ok(range.to_vec())
    }

    /// Caret offset after a keyboard movement (`left`, `word_right`,
    /// `line_up`, `line_end`, ...).
    #[wasm_bindgen(js_name = moveCaret)]
    pub fn move_caret(
        &mut self,
        page: usize,
        offset: usize,
        movement: &str,
    ) -> Result<usize, JsValue> {
        let movement = CaretMove::parse(movement)
            .ok_or_else(|| JsValue::from_str(&format!("unknown caret movement {movement}")))?;
        Ok(self.layout(page)?.move_caret(offset, movement))
    }

    /// Replace `[start, end)` with `text`; returns the outcome JSON
    /// (`caret`, `dirty_region` in user space, transaction `report`).
    pub fn replace(
        &mut self,
        page: usize,
        start: usize,
        end: usize,
        text: &str,
    ) -> Result<String, JsValue> {
        encode(
            &self
                .session
                .replace(page, [start, end], text)
                .map_err(err)?,
        )
    }

    pub fn insert(&mut self, page: usize, offset: usize, text: &str) -> Result<String, JsValue> {
        encode(&self.session.insert(page, offset, text).map_err(err)?)
    }

    pub fn delete(&mut self, page: usize, start: usize, end: usize) -> Result<String, JsValue> {
        encode(&self.session.delete(page, [start, end]).map_err(err)?)
    }

    pub fn undo(&mut self) -> bool {
        self.session.undo()
    }

    pub fn redo(&mut self) -> bool {
        self.session.redo()
    }

    #[wasm_bindgen(js_name = canUndo)]
    pub fn can_undo(&self) -> bool {
        self.session.can_undo()
    }

    #[wasm_bindgen(js_name = canRedo)]
    pub fn can_redo(&self) -> bool {
        self.session.can_redo()
    }

    /// The current document bytes.
    pub fn bytes(&self) -> Vec<u8> {
        self.session.bytes().to_vec()
    }
}
