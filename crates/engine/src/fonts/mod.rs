pub(crate) mod cid;
pub mod cmap;
pub mod encoding;
pub mod glyph_list;
pub mod predefined_cmap;
pub mod provider;
pub mod resolver;
pub(crate) mod sfnt_subset;
pub mod shaper;
pub(crate) mod type1;
pub mod variations;

pub use provider::{
    BundledFontProvider, FontMatch, FontMatchRequest, FontProvider, FontProviderSource,
};
pub use resolver::{FontDecodeSource, FontResolver, FontType};
pub use shaper::{ShapeOptions, ShapedGlyph, ShapedRun, TextDirection, TextShaper};
pub use variations::{AxisValue, VariationRequest};
