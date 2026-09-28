pub(crate) mod base_instance;
pub(crate) mod cff2_program;
pub mod character_code;
pub(crate) mod cid;
pub(crate) mod cid_encoding;
pub mod cmap;
pub(crate) mod cmap_program;
pub(crate) mod cmap_stream;
pub(crate) mod coverage;
pub(crate) mod cvar_instance;
pub mod encoding;
pub mod fallback;
pub mod font_asset;
mod font_container;
pub mod font_instance;
mod font_instance_metadata;
pub(crate) mod font_layout_instance;
pub(crate) mod font_metric_instance;
pub(crate) mod gdef_instance;
pub(crate) mod glyf_instance;
mod glyf_program;
pub mod glyph_list;
pub(crate) mod glyph_metric_instance;
pub(crate) mod glyph_metric_variations;
pub(crate) mod gpos_instance;
pub(crate) mod gvar_instance;
pub mod hard_break;
mod instance_coordinates;
pub(crate) mod jstf_instance;
pub(crate) mod layout_instance;
pub mod line_break_policy;
pub mod line_layout;
pub(crate) mod logical_carrier;
pub(crate) mod mvar_instance;
pub(crate) mod pdf_embedding;
#[cfg(test)]
pub(crate) mod pdf_embedding_fixtures;
mod position_instance;
pub mod predefined_cmap;
pub mod provider;
pub mod resolver;
pub(crate) mod sfnt_outline;
pub(crate) mod sfnt_subset;
pub mod shaper;
pub mod shaping_context;
pub mod tab_stops;
mod tt_bytecode;
pub(crate) mod tt_hint_instance;
pub(crate) mod tuple_variations;
pub(crate) mod type1;
#[cfg(test)]
pub(crate) mod variable_cmap_tests;
pub(crate) mod variation_store;
pub mod variations;
pub mod vertical;
pub mod vertical_fonts;
pub mod writing_mode;
pub use writing_mode::WritingMode;

pub use provider::{
    BundledFontProvider, FontMatch, FontMatchRequest, FontProvider, FontProviderSource,
    RegisteredFontProvider,
};
pub use resolver::{FontDecodeSource, FontResolver, FontType};
pub use shaper::{ShapeOptions, ShapedGlyph, ShapedRun, TextDirection, TextShaper};
pub use variations::{AxisValue, VariationRequest};
