//! Logical inline/block axes, independent of bidi direction and glyph rotation.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WritingMode {
    #[default]
    HorizontalTb,
    VerticalRl,
    VerticalLr,
}
impl WritingMode {
    pub fn is_vertical(self) -> bool {
        self != Self::HorizontalTb
    }
}
