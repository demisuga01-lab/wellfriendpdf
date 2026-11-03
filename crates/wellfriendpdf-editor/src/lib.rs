//! Wellfriend PDF SDK interactive editor.
//!
//! The editor model is an [`OperationList`]: text replacements in place,
//! image and vector object edits, added text/images/shapes/links/whiteouts
//! and true redactions. The browser editor applies each operation with this
//! crate compiled to WASM to preview it; the server replays the saved list
//! with the native build to produce the new immutable document version.
//! Both run [`OperationList::validate`] first.

pub mod annotations;
pub mod apply;
pub mod error;
pub mod fonts;
pub mod images;
pub mod objects;
pub mod ops;
pub mod overlay;
pub mod pages;
#[cfg(not(target_arch = "wasm32"))]
pub mod protocol;
pub mod restyle;
mod scratch;
pub mod windows;

pub use apply::{apply_operation, apply_operations, ApplyOutcome, OperationReport};
pub use error::EditorError;
pub use fonts::{FontSource, MemoryFonts, SourceFont};
pub use objects::{page_objects, PageObjects};
pub use ops::{EditorOperation, OperationList, OPERATION_LIST_VERSION};

#[cfg(not(target_arch = "wasm32"))]
pub use fonts::DirectoryFonts;
