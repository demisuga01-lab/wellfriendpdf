use serde::Serialize;

/// A refused or failed editor operation. `code` is stable for clients;
/// `operation` is the index in the operation list when one is to blame.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<usize>,
}

impl EditorError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
            operation: None,
        }
    }

    pub fn at(mut self, index: usize) -> Self {
        if self.operation.is_none() {
            self.operation = Some(index);
        }
        self
    }

    pub fn engine(error: impl std::fmt::Display) -> Self {
        Self::new("EDITOR_ENGINE_REFUSED", error.to_string())
    }

    pub fn edit(error: wellfriend_pdf_edit::PdfEditError) -> Self {
        Self::new("EDITOR_OBJECT_EDIT_REFUSED", format!("{error}"))
    }
}

impl std::fmt::Display for EditorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.operation {
            Some(index) => write!(f, "{} (operation {}): {}", self.code, index, self.message),
            None => write!(f, "{}: {}", self.code, self.message),
        }
    }
}

impl std::error::Error for EditorError {}

impl From<wellfriendpdf_engine::WellfriendError> for EditorError {
    fn from(error: wellfriendpdf_engine::WellfriendError) -> Self {
        Self::engine(error)
    }
}
