//! Versioned, binding-safe render contract.
//!
//! This module keeps every public pixel-affecting choice in one immutable,
//! serializable value. Callers that request an option outside the selected
//! backend policy receive a typed error instead of silently getting a different
//! rendering policy.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Result, WellfriendError};

use super::buffer::RenderMode;
use super::display_list::RenderTile;
use super::transform::Viewport;

pub const RENDER_CONTRACT_SCHEMA_VERSION: u32 = 1;

#[repr(transparent)]
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct RevisionId(pub u64);

#[repr(transparent)]
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct ObjectIdentityId(pub u32);

#[repr(transparent)]
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct ResourceId(pub u32);

#[repr(transparent)]
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct SourceLinkId(pub u32);

#[repr(transparent)]
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct DisplayItemId(pub u32);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PageBox {
    Media,
    #[default]
    Crop,
    Bleed,
    Trim,
    Art,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PixelFormat {
    #[default]
    Rgba8,
    Bgra8,
    Rgb8,
    Bgr8,
    Gray8,
    /// Interleaved C, M, Y, K ink bytes (0 = no ink), composited natively
    /// in CMYK by the plate renderer (Poppler `splashModeCMYK8`). Alpha mode,
    /// grayscale and byte-order flags do not apply.
    Cmyk8,
}

impl PixelFormat {
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Rgba8 | Self::Bgra8 | Self::Cmyk8 => 4,
            Self::Rgb8 | Self::Bgr8 => 3,
            Self::Gray8 => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AlphaMode {
    #[default]
    Premultiplied,
    Straight,
    Opaque,
}

/// Render-contract execution mode. Standard is the only mode; the retired
/// `Research` value is still accepted on input as a deprecated alias.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExecutionMode {
    #[default]
    #[serde(alias = "Research", alias = "research", alias = "standard")]
    Standard,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BackendSelection {
    ScalarReference,
    #[default]
    StandardCpu,
    /// Guarded CPU hybrid dispatch with its own contract/decode cache
    /// identity. Formerly `ResearchHybrid`, which is accepted as an alias.
    #[serde(alias = "ResearchHybrid")]
    Hybrid,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SmoothingPolicy {
    Disabled,
    #[default]
    Antialiased,
    Subpixel,
}

/// Treatment of strokes thinner than one device pixel, mirroring
/// `pdftoppm -thinlinemode`.
///
/// * `None` (default): the regular rasterizer behaviour. Strokes keep a
///   minimum device width of one pixel and are anti-aliased (and, where stroke
///   adjustment applies, snapped to whole device pixels).
/// * `Solid`: strokes thinner than one device pixel (including zero-width
///   hairlines) are drawn as fully opaque one-pixel lines; their coverage is
///   thresholded instead of anti-aliased.
/// * `Shape`: strokes thinner than one device pixel keep the one-pixel
///   footprint but their opacity is scaled by their true device width, so a
///   0.25 px rule renders at 25% opacity instead of as a full pixel. Zero-width
///   hairlines are unaffected (they have no width to scale by).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ThinLineMode {
    #[default]
    None,
    Solid,
    Shape,
}

/// Mapping from geometric anti-aliasing coverage to paint opacity for vector
/// fills and strokes (glyphs, images and clips are unaffected).
///
/// * `Linear` (default, PDFium/Skia convention): opacity equals coverage.
/// * `Gamma15` (Poppler/Splash convention): opacity is coverage raised to the
///   power 1.5, so partially covered edge pixels are lighter (a half-covered
///   pixel paints about 35% instead of 50%) and vector edges look crisper.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VectorAntialiasGamma {
    #[default]
    Linear,
    Gamma15,
}

/// Mapping from glyph coverage to paint opacity for text drawn from glyph
/// masks (paths, images and clips are unaffected).
///
/// * `Linear` (default): opacity equals glyph coverage.
/// * `Pdfium`: PDFium's 256-entry text gamma table (`kTextGammaAdjust`),
///   which darkens partially covered glyph pixels before blending. With LCD
///   text it is applied to each colour channel's coverage separately.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TextGamma {
    #[default]
    Linear,
    Pdfium,
}

impl TextGamma {
    pub fn is_linear(&self) -> bool {
        *self == TextGamma::Linear
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AnnotationRenderPolicy {
    #[default]
    Include,
    Exclude,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FormRenderPolicy {
    #[default]
    Include,
    Exclude,
}

/// XFA form drawing (PDFium `kXFAForeground`). `Auto` (the default) draws
/// the XFA fields of a static XFA form over a page only when that page has
/// no widget annotations or a widget without a normal appearance; PDFs
/// without XFA are never affected. `Foreground` always draws the XFA layout
/// over the PDF pages of any XFA document (dynamic forms included); `Off`
/// never does. Widget annotations of a page with an XFA overlay are not
/// drawn: the XFA fields replace them. Dynamic forms' own pages are
/// rendered with `ContentEngine::render_xfa_page`. Omitted from the
/// serialized contract while `Auto`, so existing cache identities stay
/// stable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum XfaRenderPolicy {
    #[default]
    Auto,
    Foreground,
    Off,
}

impl XfaRenderPolicy {
    pub fn is_auto(&self) -> bool {
        *self == XfaRenderPolicy::Auto
    }
}

/// Viewer-style form-field highlighting, like PDFium's
/// `FPDF_SetFormFieldHighlightColor` / `FPDF_SetFormFieldHighlightAlpha`:
/// a translucent colour painted over every visible, non-read-only widget of
/// a field type after its appearance, plus an optional dashed focus
/// rectangle around the widgets of one field. Off by default; the default
/// value does not change rendering or the contract's cache identity.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct FormHighlightPolicy {
    /// Text fields (`/FT /Tx`). The colour's `a` is the highlight opacity;
    /// `None` or `a == 0` leaves the field type unhighlighted.
    pub text: Option<ContractColor>,
    /// Push buttons.
    pub push_button: Option<ContractColor>,
    /// Check boxes.
    pub check_box: Option<ContractColor>,
    /// Radio buttons.
    pub radio_button: Option<ContractColor>,
    /// Combo boxes.
    pub combo_box: Option<ContractColor>,
    /// List boxes.
    pub list_box: Option<ContractColor>,
    /// Signature fields.
    pub signature: Option<ContractColor>,
    /// Fully qualified name (`parent.child`) of the focused field: its
    /// widgets get PDFium's black dotted focus rectangle, 1pt outside the
    /// widget rectangle.
    pub focus_field: Option<String>,
}

impl FormHighlightPolicy {
    /// The same colour for every field type, like
    /// `FPDF_SetFormFieldHighlightColor(handle, 0, color)`.
    pub fn all(color: ContractColor) -> Self {
        Self {
            text: Some(color),
            push_button: Some(color),
            check_box: Some(color),
            radio_button: Some(color),
            combo_box: Some(color),
            list_box: Some(color),
            signature: Some(color),
            focus_field: None,
        }
    }

    /// True when nothing is highlighted and no focus rectangle is drawn.
    pub fn is_off(&self) -> bool {
        let off = |color: &Option<ContractColor>| color.is_none_or(|color| color.a == 0);
        off(&self.text)
            && off(&self.push_button)
            && off(&self.check_box)
            && off(&self.radio_button)
            && off(&self.combo_box)
            && off(&self.list_box)
            && off(&self.signature)
            && self.focus_field.as_deref().is_none_or(str::is_empty)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ColorScheme {
    #[default]
    Light,
    Dark,
    ForcedMonochrome,
}

/// Forced (high-contrast / accessibility) colour scheme, the equivalent of
/// PDFium's `FPDF_COLORSCHEME` with `FPDF_CONVERT_FILL_TO_STROKE`.
///
/// When set on a [`RenderContract`], path and text fill/stroke colours are
/// replaced by these straight-alpha RGBA colours while images, shadings,
/// pattern paints and soft-mask groups keep their real colours. As in PDFium
/// the scheme colour (including its alpha) replaces the resolved paint
/// colour entirely; graphics-state constant alpha and transfer functions do
/// not modify it. With `convert_fill_to_stroke`, filled paths are drawn as
/// strokes of their current line width (at least one device pixel) in the
/// path stroke colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ForcedColorScheme {
    pub path_fill: ContractColor,
    pub path_stroke: ContractColor,
    pub text_fill: ContractColor,
    pub text_stroke: ContractColor,
    #[serde(default)]
    pub convert_fill_to_stroke: bool,
}

impl ForcedColorScheme {
    /// A high-contrast scheme: one foreground colour for every path and text
    /// paint, without fill-to-stroke conversion.
    pub fn uniform(foreground: ContractColor) -> Self {
        Self {
            path_fill: foreground,
            path_stroke: foreground,
            text_fill: foreground,
            text_stroke: foreground,
            convert_fill_to_stroke: false,
        }
    }
}

impl ContractColor {
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn to_pixel(self) -> [u8; 4] {
        [self.r, self.g, self.b, self.a]
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PrintProfile {
    #[default]
    Display,
    Print,
    Proof,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HalftonePolicy {
    #[default]
    Disabled,
    Screen,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OverprintPolicy {
    #[default]
    Disabled,
    Preview,
    PreserveSeparations,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RenderingIntent {
    #[default]
    RelativeColorimetric,
    AbsoluteColorimetric,
    Perceptual,
    Saturation,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ColorManagementPolicy {
    #[default]
    PortableQcms,
    NativeLittleCms,
    DeterministicFallback,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExactnessPolicy {
    #[default]
    Compatibility,
    HighQualityExact,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DeterminismPolicy {
    #[default]
    Required,
    /// Best-effort determinism. Formerly `BestEffortResearch`, which is
    /// accepted as an alias.
    #[serde(alias = "BestEffortResearch")]
    BestEffort,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CompositingPolicy {
    #[default]
    Compatibility,
    HighQuality,
}

impl From<RenderMode> for CompositingPolicy {
    fn from(value: RenderMode) -> Self {
        match value {
            RenderMode::Compat => Self::Compatibility,
            RenderMode::HighQuality => Self::HighQuality,
        }
    }
}

impl From<CompositingPolicy> for RenderMode {
    fn from(value: CompositingPolicy) -> Self {
        match value {
            CompositingPolicy::Compatibility => Self::Compat,
            CompositingPolicy::HighQuality => Self::HighQuality,
        }
    }
}

/// Matrix components stored as IEEE-754 bit patterns so the public contract is
/// hashable and portable without silently canonicalizing NaN payloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeviceMatrix {
    pub values: [u64; 6],
}

impl Default for DeviceMatrix {
    fn default() -> Self {
        Self::from_f64([1.0, 0.0, 0.0, 1.0, 0.0, 0.0])
    }
}

impl DeviceMatrix {
    pub fn from_f64(values: [f64; 6]) -> Self {
        Self {
            values: values.map(f64::to_bits),
        }
    }

    pub fn to_f64(self) -> [f64; 6] {
        self.values.map(f64::from_bits)
    }

    pub fn is_identity(self) -> bool {
        self == Self::default()
    }

    fn is_finite(self) -> bool {
        self.to_f64().iter().all(|value| value.is_finite())
    }

    fn is_invertible(self) -> bool {
        let [a, b, c, d, _, _] = self.to_f64();
        (a * d - b * c).abs() >= 1e-10
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeviceClip {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContractColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Default for ContractColor {
    fn default() -> Self {
        Self {
            r: 255,
            g: 255,
            b: 255,
            a: 255,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OptionalContentStateId(pub String);

impl From<String> for OptionalContentStateId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl Default for OptionalContentStateId {
    fn default() -> Self {
        Self("ocg:default".to_string())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RenderResourceBudget {
    pub max_pixels: u64,
    pub max_decoded_bytes: u64,
    pub max_temporary_bytes: u64,
    pub max_cache_bytes: u64,
}

impl Default for RenderResourceBudget {
    fn default() -> Self {
        Self {
            max_pixels: 100_000_000,
            max_decoded_bytes: 512 * 1024 * 1024,
            max_temporary_bytes: 256 * 1024 * 1024,
            max_cache_bytes: 256 * 1024 * 1024,
        }
    }
}

/// Source-owned parity map for every public render-contract field.
///
/// This registry is intentionally kept next to [`RenderContract`] and is
/// checked against serde field names by unit tests. If a new public contract
/// field is added, its cache/execution/binding posture must be recorded here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct RenderContractFieldEffect {
    pub field: &'static str,
    pub cache_identity: bool,
    pub validation: bool,
    pub active_execution: bool,
    pub binding_builder: bool,
    pub effect: &'static str,
}

pub const RENDER_CONTRACT_FIELD_EFFECTS: &[RenderContractFieldEffect] = &[
    RenderContractFieldEffect {
        field: "schema_version",
        cache_identity: true,
        validation: true,
        active_execution: true,
        binding_builder: false,
        effect: "schema-refusal-and-transport-version",
    },
    RenderContractFieldEffect {
        field: "document_revision",
        cache_identity: true,
        validation: true,
        active_execution: true,
        binding_builder: false,
        effect: "document-revision-cache-and-mismatch-refusal",
    },
    RenderContractFieldEffect {
        field: "page_identity",
        cache_identity: true,
        validation: false,
        active_execution: true,
        binding_builder: false,
        effect: "page-object-cache-identity",
    },
    RenderContractFieldEffect {
        field: "page_number",
        cache_identity: true,
        validation: true,
        active_execution: true,
        binding_builder: true,
        effect: "page-selection-and-viewport",
    },
    RenderContractFieldEffect {
        field: "dpi",
        cache_identity: true,
        validation: false,
        active_execution: true,
        binding_builder: true,
        effect: "viewport-scale-and-output-dimensions",
    },
    RenderContractFieldEffect {
        field: "page_box",
        cache_identity: true,
        validation: false,
        active_execution: true,
        binding_builder: true,
        effect: "box-selection-and-cache-identity",
    },
    RenderContractFieldEffect {
        field: "transform",
        cache_identity: true,
        validation: true,
        active_execution: true,
        binding_builder: true,
        effect: "device-transform-render-policy",
    },
    RenderContractFieldEffect {
        field: "clip",
        cache_identity: true,
        validation: true,
        active_execution: true,
        binding_builder: true,
        effect: "tile-selection-cropping-and-budgeting",
    },
    RenderContractFieldEffect {
        field: "width",
        cache_identity: true,
        validation: true,
        active_execution: true,
        binding_builder: true,
        effect: "surface-dimensions-and-buffer-validation",
    },
    RenderContractFieldEffect {
        field: "height",
        cache_identity: true,
        validation: true,
        active_execution: true,
        binding_builder: true,
        effect: "surface-dimensions-and-buffer-validation",
    },
    RenderContractFieldEffect {
        field: "stride",
        cache_identity: true,
        validation: true,
        active_execution: true,
        binding_builder: true,
        effect: "caller-surface-row-layout",
    },
    RenderContractFieldEffect {
        field: "pixel_format",
        cache_identity: true,
        validation: true,
        active_execution: true,
        binding_builder: true,
        effect: "caller-surface-channel-layout",
    },
    RenderContractFieldEffect {
        field: "alpha_mode",
        cache_identity: true,
        validation: false,
        active_execution: true,
        binding_builder: true,
        effect: "caller-surface-alpha-conversion",
    },
    RenderContractFieldEffect {
        field: "background",
        cache_identity: true,
        validation: false,
        active_execution: true,
        binding_builder: true,
        effect: "clear-color-and-background-flattening",
    },
    RenderContractFieldEffect {
        field: "execution_mode",
        cache_identity: true,
        validation: false,
        active_execution: true,
        binding_builder: true,
        effect: "standard-policy-identity",
    },
    RenderContractFieldEffect {
        field: "backend",
        cache_identity: true,
        validation: false,
        active_execution: true,
        binding_builder: true,
        effect: "scalar-standard-or-hybrid-dispatch",
    },
    RenderContractFieldEffect {
        field: "compositing",
        cache_identity: true,
        validation: false,
        active_execution: true,
        binding_builder: true,
        effect: "compatibility-or-high-quality-render-mode",
    },
    RenderContractFieldEffect {
        field: "annotations",
        cache_identity: true,
        validation: false,
        active_execution: true,
        binding_builder: true,
        effect: "annotation-appearance-inclusion",
    },
    RenderContractFieldEffect {
        field: "forms",
        cache_identity: true,
        validation: false,
        active_execution: true,
        binding_builder: true,
        effect: "form-widget-appearance-inclusion",
    },
    RenderContractFieldEffect {
        field: "form_highlight",
        cache_identity: true,
        validation: false,
        active_execution: true,
        binding_builder: true,
        effect: "form-field-highlight-and-focus-rectangle",
    },
    RenderContractFieldEffect {
        field: "optional_content",
        cache_identity: true,
        validation: true,
        active_execution: true,
        binding_builder: true,
        effect: "optional-content-state-resolution",
    },
    RenderContractFieldEffect {
        field: "text_smoothing",
        cache_identity: true,
        validation: false,
        active_execution: true,
        binding_builder: true,
        effect: "text-rasterization-smoothing-policy",
    },
    RenderContractFieldEffect {
        field: "image_smoothing",
        cache_identity: true,
        validation: false,
        active_execution: true,
        binding_builder: true,
        effect: "image-sampling-policy",
    },
    RenderContractFieldEffect {
        field: "path_smoothing",
        cache_identity: true,
        validation: false,
        active_execution: true,
        binding_builder: true,
        effect: "path-scan-conversion-policy",
    },
    RenderContractFieldEffect {
        field: "subpixel_text",
        cache_identity: true,
        validation: false,
        active_execution: true,
        binding_builder: true,
        effect: "subpixel-text-policy",
    },
    RenderContractFieldEffect {
        field: "thin_line_mode",
        cache_identity: true,
        validation: false,
        active_execution: true,
        binding_builder: true,
        effect: "thin-stroke-solid-or-shape-policy",
    },
    RenderContractFieldEffect {
        field: "vector_antialias_gamma",
        cache_identity: true,
        validation: false,
        active_execution: true,
        binding_builder: true,
        effect: "vector-coverage-to-opacity-curve",
    },
    RenderContractFieldEffect {
        field: "xfa",
        cache_identity: true,
        validation: false,
        active_execution: true,
        binding_builder: false,
        effect: "xfa-foreground-form-overlay-policy",
    },
    RenderContractFieldEffect {
        field: "text_gamma",
        cache_identity: true,
        validation: false,
        active_execution: true,
        binding_builder: false,
        effect: "glyph-coverage-text-gamma-table",
    },
    RenderContractFieldEffect {
        field: "grayscale",
        cache_identity: true,
        validation: false,
        active_execution: true,
        binding_builder: true,
        effect: "caller-surface-grayscale-conversion",
    },
    RenderContractFieldEffect {
        field: "color_scheme",
        cache_identity: true,
        validation: false,
        active_execution: true,
        binding_builder: true,
        effect: "post-render-color-scheme-transform",
    },
    RenderContractFieldEffect {
        field: "forced_colors",
        cache_identity: true,
        validation: false,
        active_execution: true,
        binding_builder: true,
        effect: "forced-path-and-text-paint-colors-and-fill-to-stroke",
    },
    RenderContractFieldEffect {
        field: "reverse_byte_order",
        cache_identity: true,
        validation: false,
        active_execution: true,
        binding_builder: true,
        effect: "caller-surface-byte-order-routing",
    },
    RenderContractFieldEffect {
        field: "print_profile",
        cache_identity: true,
        validation: true,
        active_execution: true,
        binding_builder: true,
        effect: "display-print-proof-profile-selection",
    },
    RenderContractFieldEffect {
        field: "halftone",
        cache_identity: true,
        validation: true,
        active_execution: true,
        binding_builder: true,
        effect: "ordered-halftone-post-process",
    },
    RenderContractFieldEffect {
        field: "overprint",
        cache_identity: true,
        validation: true,
        active_execution: true,
        binding_builder: true,
        effect: "overprint-preview-or-typed-separation-refusal",
    },
    RenderContractFieldEffect {
        field: "rendering_intent",
        cache_identity: true,
        validation: false,
        active_execution: true,
        binding_builder: true,
        effect: "color-management-rendering-intent",
    },
    RenderContractFieldEffect {
        field: "color_management",
        cache_identity: true,
        validation: true,
        active_execution: true,
        binding_builder: true,
        effect: "portable-native-or-deterministic-cmm-policy",
    },
    RenderContractFieldEffect {
        field: "exactness",
        cache_identity: true,
        validation: false,
        active_execution: true,
        binding_builder: true,
        effect: "high-quality-exact-refusal-policy",
    },
    RenderContractFieldEffect {
        field: "determinism",
        cache_identity: true,
        validation: false,
        active_execution: true,
        binding_builder: true,
        effect: "deterministic-output-policy",
    },
    RenderContractFieldEffect {
        field: "resource_budget",
        cache_identity: true,
        validation: true,
        active_execution: true,
        binding_builder: true,
        effect: "pixel-decode-temporary-and-cache-budgeting",
    },
];

pub fn render_contract_field_effects() -> &'static [RenderContractFieldEffect] {
    RENDER_CONTRACT_FIELD_EFFECTS
}

/// A complete, versioned request for raster semantics. Fields may be rejected
/// by a backend when its implementation cannot honor them exactly; they are
/// never omitted from cache identity.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RenderContract {
    pub schema_version: u32,
    pub document_revision: RevisionId,
    pub page_identity: ObjectIdentityId,
    pub page_number: usize,
    pub dpi: u32,
    pub page_box: PageBox,
    pub transform: DeviceMatrix,
    pub clip: Option<DeviceClip>,
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub pixel_format: PixelFormat,
    pub alpha_mode: AlphaMode,
    pub background: ContractColor,
    pub execution_mode: ExecutionMode,
    pub backend: BackendSelection,
    pub compositing: CompositingPolicy,
    pub annotations: AnnotationRenderPolicy,
    pub forms: FormRenderPolicy,
    /// Form-field highlight and focus rectangle (off by default; omitted
    /// from the serialized contract while off).
    #[serde(default, skip_serializing_if = "FormHighlightPolicy::is_off")]
    pub form_highlight: FormHighlightPolicy,
    /// XFA foreground drawing (see [`XfaRenderPolicy`]).
    #[serde(default, skip_serializing_if = "XfaRenderPolicy::is_auto")]
    pub xfa: XfaRenderPolicy,
    pub optional_content: OptionalContentStateId,
    pub text_smoothing: SmoothingPolicy,
    pub image_smoothing: SmoothingPolicy,
    pub path_smoothing: SmoothingPolicy,
    pub subpixel_text: SmoothingPolicy,
    /// Thin-stroke policy (`pdftoppm -thinlinemode`). Older serialized
    /// contracts without this field deserialize to [`ThinLineMode::None`].
    #[serde(default)]
    pub thin_line_mode: ThinLineMode,
    /// Vector anti-aliasing coverage curve. Older serialized contracts
    /// without this field deserialize to [`VectorAntialiasGamma::Linear`].
    #[serde(default)]
    pub vector_antialias_gamma: VectorAntialiasGamma,
    /// Glyph coverage gamma (PDFium text gamma table). Off by default; the
    /// field is omitted from the serialized contract while linear so existing
    /// cache identities stay stable.
    #[serde(default, skip_serializing_if = "TextGamma::is_linear")]
    pub text_gamma: TextGamma,
    pub grayscale: bool,
    pub color_scheme: ColorScheme,
    /// Optional forced (high-contrast) paint colours for paths and text.
    /// `None` (the default) leaves rendering unchanged; the field is omitted
    /// from the serialized contract while unset so existing cache identities
    /// stay stable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forced_colors: Option<ForcedColorScheme>,
    pub reverse_byte_order: bool,
    pub print_profile: PrintProfile,
    pub halftone: HalftonePolicy,
    pub overprint: OverprintPolicy,
    pub rendering_intent: RenderingIntent,
    pub color_management: ColorManagementPolicy,
    pub exactness: ExactnessPolicy,
    pub determinism: DeterminismPolicy,
    pub resource_budget: RenderResourceBudget,
}

impl RenderContract {
    pub fn for_viewport(
        revision: RevisionId,
        page_identity: ObjectIdentityId,
        page_number: usize,
        viewport: &Viewport,
        tile: RenderTile,
        render_mode: RenderMode,
    ) -> Self {
        let width = tile.width;
        let height = tile.height;
        Self {
            schema_version: RENDER_CONTRACT_SCHEMA_VERSION,
            document_revision: revision,
            page_identity,
            page_number,
            dpi: viewport.dpi,
            page_box: PageBox::Crop,
            transform: DeviceMatrix::default(),
            clip: Some(DeviceClip {
                x: i32::try_from(tile.x).unwrap_or(i32::MAX),
                y: i32::try_from(tile.y).unwrap_or(i32::MAX),
                width,
                height,
            }),
            width,
            height,
            stride: width as usize * PixelFormat::Rgba8.bytes_per_pixel(),
            pixel_format: PixelFormat::Rgba8,
            alpha_mode: AlphaMode::Premultiplied,
            background: ContractColor::default(),
            execution_mode: ExecutionMode::Standard,
            backend: BackendSelection::StandardCpu,
            compositing: render_mode.into(),
            annotations: AnnotationRenderPolicy::Include,
            forms: FormRenderPolicy::Include,
            form_highlight: FormHighlightPolicy::default(),
            xfa: XfaRenderPolicy::Auto,
            optional_content: OptionalContentStateId::default(),
            text_smoothing: SmoothingPolicy::Antialiased,
            image_smoothing: SmoothingPolicy::Antialiased,
            path_smoothing: SmoothingPolicy::Antialiased,
            subpixel_text: SmoothingPolicy::Disabled,
            thin_line_mode: ThinLineMode::None,
            vector_antialias_gamma: VectorAntialiasGamma::Linear,
            text_gamma: TextGamma::Linear,
            grayscale: false,
            color_scheme: ColorScheme::Light,
            forced_colors: None,
            reverse_byte_order: false,
            print_profile: PrintProfile::Display,
            halftone: HalftonePolicy::Disabled,
            overprint: OverprintPolicy::Disabled,
            rendering_intent: RenderingIntent::RelativeColorimetric,
            color_management: ColorManagementPolicy::PortableQcms,
            exactness: if render_mode.is_high_quality() {
                ExactnessPolicy::HighQualityExact
            } else {
                ExactnessPolicy::Compatibility
            },
            determinism: DeterminismPolicy::Required,
            resource_budget: RenderResourceBudget::default(),
        }
    }

    pub fn render_mode(&self) -> RenderMode {
        self.compositing.into()
    }

    pub fn with_render_tile(&self, tile: RenderTile) -> Self {
        let mut contract = self.clone();
        contract.clip = Some(DeviceClip {
            x: i32::try_from(tile.x).unwrap_or(i32::MAX),
            y: i32::try_from(tile.y).unwrap_or(i32::MAX),
            width: tile.width,
            height: tile.height,
        });
        contract.width = tile.width;
        contract.height = tile.height;
        contract.stride = tile.width as usize * contract.pixel_format.bytes_per_pixel();
        contract
    }

    pub fn cache_fingerprint(&self) -> String {
        let bytes = serde_json::to_vec(self).expect("RenderContract serialization is infallible");
        let digest = Sha256::digest(bytes);
        digest.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    pub fn is_default_geometry(&self) -> bool {
        self.transform.is_identity()
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != RENDER_CONTRACT_SCHEMA_VERSION {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "render contract schema {} is unsupported; expected {}",
                self.schema_version, RENDER_CONTRACT_SCHEMA_VERSION
            )));
        }
        if self.page_number == 0 {
            return Err(WellfriendError::invalid_input(
                "render contract page_number must be 1-based",
            ));
        }
        if self.width == 0 || self.height == 0 {
            return Err(WellfriendError::invalid_input(
                "render contract output width and height must be non-zero",
            ));
        }
        if !self.transform.is_finite() {
            return Err(WellfriendError::invalid_input(
                "render contract transform must contain only finite values",
            ));
        }
        if !self.transform.is_invertible() {
            return Err(WellfriendError::invalid_input(
                "render contract transform must be invertible",
            ));
        }
        let minimum_stride = self.width as usize * self.pixel_format.bytes_per_pixel();
        if self.stride < minimum_stride {
            return Err(WellfriendError::invalid_input(format!(
                "render contract stride {} is below the required {} bytes",
                self.stride, minimum_stride
            )));
        }
        let pixels = u64::from(self.width) * u64::from(self.height);
        if pixels > self.resource_budget.max_pixels {
            return Err(WellfriendError::ResourceLimit(format!(
                "render contract requests {pixels} pixels, exceeding budget {}",
                self.resource_budget.max_pixels
            )));
        }
        let temporary_surface_bytes = pixels.checked_mul(4).ok_or_else(|| {
            WellfriendError::ResourceLimit(
                "render contract temporary surface byte length overflows".to_string(),
            )
        })?;
        if temporary_surface_bytes > self.resource_budget.max_temporary_bytes {
            return Err(WellfriendError::ResourceLimit(format!(
                "render contract requires {temporary_surface_bytes} temporary bytes for the canonical RGBA working surface, exceeding max_temporary_bytes {}",
                self.resource_budget.max_temporary_bytes
            )));
        }
        if let Some(clip) = self.clip {
            if clip.width == 0 || clip.height == 0 {
                return Err(WellfriendError::invalid_input(
                    "render contract clip must have non-zero dimensions",
                ));
            }
        }
        // Validate that the print profile + prepress combination is implementable.
        if let Err(refusal) = super::print_profile::validate_print_profile_prepress(
            self.print_profile,
            self.halftone,
            self.overprint,
            self.color_management,
        ) {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "render contract print profile refusal: {} (category: {:?})",
                refusal.reason, refusal.category
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contract(revision: u64) -> RenderContract {
        let viewport = Viewport::new([0.0, 0.0, 10.0, 10.0], 72);
        RenderContract::for_viewport(
            RevisionId(revision),
            ObjectIdentityId(1),
            1,
            &viewport,
            RenderTile::full(10, 10),
            RenderMode::Compat,
        )
    }

    #[test]
    fn revision_and_policy_change_cache_identity() {
        let first = contract(1);
        let mut second = contract(2);
        assert_ne!(first.cache_fingerprint(), second.cache_fingerprint());
        second.document_revision = first.document_revision;
        second.annotations = AnnotationRenderPolicy::Exclude;
        assert_ne!(first.cache_fingerprint(), second.cache_fingerprint());
    }

    #[test]
    fn contract_rejects_unknown_schema_and_short_stride() {
        let mut contract = contract(1);
        contract.schema_version += 1;
        assert!(contract.validate().is_err());
        contract.schema_version = RENDER_CONTRACT_SCHEMA_VERSION;
        contract.stride = 1;
        assert!(contract.validate().is_err());
    }

    #[test]
    fn contract_rejects_singular_device_transform() {
        let mut contract = contract(1);
        contract.transform = DeviceMatrix::from_f64([1.0, 0.0, 1.0, 0.0, 0.0, 0.0]);

        let err = contract
            .validate()
            .expect_err("singular transform must be refused");
        assert!(err.to_string().contains("transform must be invertible"));
    }

    #[test]
    fn legacy_research_contract_values_deserialize_to_merged_variants() {
        let base = contract(3);
        let mut legacy = serde_json::to_value(&base).expect("serialize contract");
        legacy["execution_mode"] = "Research".into();
        legacy["backend"] = "ResearchHybrid".into();
        legacy["determinism"] = "BestEffortResearch".into();
        let parsed: RenderContract =
            serde_json::from_value(legacy).expect("legacy research contract deserializes");
        assert_eq!(parsed.execution_mode, ExecutionMode::Standard);
        assert_eq!(parsed.backend, BackendSelection::Hybrid);
        assert_eq!(parsed.determinism, DeterminismPolicy::BestEffort);
        parsed.validate().expect("legacy contract stays valid");

        // A legacy `Research` execution mode now shares Standard's identity.
        let mut research_only = serde_json::to_value(&base).expect("serialize contract");
        research_only["execution_mode"] = "Research".into();
        let research_only: RenderContract =
            serde_json::from_value(research_only).expect("legacy execution mode");
        assert_eq!(research_only.cache_fingerprint(), base.cache_fingerprint());

        // New names serialize; the legacy names are input-only aliases.
        let json = serde_json::to_value(&parsed).expect("serialize merged contract");
        assert_eq!(json["execution_mode"], "Standard");
        assert_eq!(json["backend"], "Hybrid");
        assert_eq!(json["determinism"], "BestEffort");
        for (value, expected) in [
            ("\"Standard\"", ExecutionMode::Standard),
            ("\"Research\"", ExecutionMode::Standard),
            ("\"research\"", ExecutionMode::Standard),
        ] {
            assert_eq!(
                serde_json::from_str::<ExecutionMode>(value).unwrap(),
                expected
            );
        }
        assert!(serde_json::from_str::<ExecutionMode>("\"Turbo\"").is_err());
        assert_ne!(
            base.cache_fingerprint(),
            RenderContract {
                backend: BackendSelection::Hybrid,
                ..base.clone()
            }
            .cache_fingerprint(),
            "Hybrid keeps a distinct contract identity"
        );
    }

    #[test]
    fn form_highlight_is_off_by_default_and_changes_identity_when_on() {
        let base = contract(1);
        assert!(base.form_highlight.is_off());
        let json = serde_json::to_value(&base).expect("serialize contract");
        assert!(json.get("form_highlight").is_none(), "off: not serialized");
        let parsed: RenderContract = serde_json::from_value(json).expect("deserialize");
        assert_eq!(parsed, base);

        let mut highlighted = base.clone();
        highlighted.form_highlight = FormHighlightPolicy::all(ContractColor {
            r: 0,
            g: 0,
            b: 255,
            a: 80,
        });
        assert!(!highlighted.form_highlight.is_off());
        assert_ne!(highlighted.cache_fingerprint(), base.cache_fingerprint());
        let json = serde_json::to_value(&highlighted).expect("serialize contract");
        assert_eq!(json["form_highlight"]["text"]["a"], 80);
        let parsed: RenderContract = serde_json::from_value(json).expect("deserialize");
        assert_eq!(parsed, highlighted);

        let mut transparent = base.clone();
        transparent.form_highlight.text = Some(ContractColor {
            r: 0,
            g: 0,
            b: 255,
            a: 0,
        });
        assert!(transparent.form_highlight.is_off());
    }

    #[test]
    fn defaults_are_deterministic_and_valid() {
        let contract = contract(7);
        contract.validate().expect("default contract is valid");
        assert_eq!(contract.render_mode(), RenderMode::Compat);
        assert_eq!(contract.cache_fingerprint(), contract.cache_fingerprint());
    }

    #[test]
    fn print_profile_changes_cache_fingerprint_in_contract() {
        let mut display = contract(1);
        display.print_profile = PrintProfile::Display;
        let mut print = contract(1);
        print.print_profile = PrintProfile::Print;
        let mut proof = contract(1);
        proof.print_profile = PrintProfile::Proof;
        assert_ne!(display.cache_fingerprint(), print.cache_fingerprint());
        assert_ne!(print.cache_fingerprint(), proof.cache_fingerprint());
    }

    #[test]
    fn halftone_screen_is_valid_without_preserve_separations() {
        let mut c = contract(1);
        c.halftone = HalftonePolicy::Screen;
        assert!(c.validate().is_ok());
    }

    #[test]
    fn proof_with_deterministic_fallback_is_refused() {
        let mut c = contract(1);
        c.print_profile = PrintProfile::Proof;
        c.color_management = ColorManagementPolicy::DeterministicFallback;
        assert!(c.validate().is_err());
    }

    #[test]
    fn preserve_separations_is_refused_without_separation_output_surface() {
        let mut c = contract(1);
        c.overprint = OverprintPolicy::PreserveSeparations;
        c.color_management = ColorManagementPolicy::PortableQcms;
        assert!(c.validate().is_err());

        c.color_management = ColorManagementPolicy::NativeLittleCms;
        let err = c
            .validate()
            .expect_err("native CMM alone must not admit PreserveSeparations");
        assert!(err.to_string().contains("separation-preserving output"));
    }

    #[test]
    fn native_littlecms_policy_without_backend_is_refused() {
        let mut c = contract(1);
        c.color_management = ColorManagementPolicy::NativeLittleCms;
        let result = c.validate();
        if crate::render::cmm::native_cmm_status().available {
            assert!(result.is_ok());
        } else {
            let err = result.expect_err("unavailable NativeLittleCms must be refused");
            let message = err.to_string();
            assert!(message.contains("NativeLittleCms"));
            assert!(message.contains("native lcms2 backend"));
        }
    }

    #[test]
    fn raster_quality_options_default_and_change_cache_identity() {
        let base = contract(1);
        assert_eq!(base.thin_line_mode, ThinLineMode::None);
        assert_eq!(base.vector_antialias_gamma, VectorAntialiasGamma::Linear);
        let mut thin = base.clone();
        thin.thin_line_mode = ThinLineMode::Shape;
        assert_ne!(base.cache_fingerprint(), thin.cache_fingerprint());
        let mut gamma = base.clone();
        gamma.vector_antialias_gamma = VectorAntialiasGamma::Gamma15;
        assert_ne!(base.cache_fingerprint(), gamma.cache_fingerprint());

        // Contracts serialized before these options existed stay readable.
        let mut legacy = serde_json::to_value(&base).expect("serialize contract");
        let object = legacy.as_object_mut().expect("contract object");
        object.remove("thin_line_mode");
        object.remove("vector_antialias_gamma");
        let parsed: RenderContract = serde_json::from_value(legacy).expect("legacy contract");
        assert_eq!(parsed, base);
    }

    #[test]
    fn text_gamma_changes_identity_only_when_set() {
        let base = contract(1);
        assert_eq!(base.text_gamma, TextGamma::Linear);
        let json = serde_json::to_value(&base).expect("serialize contract");
        assert!(
            json.get("text_gamma").is_none(),
            "linear text gamma keeps the legacy serialized identity"
        );
        let mut gamma = base.clone();
        gamma.text_gamma = TextGamma::Pdfium;
        assert_ne!(base.cache_fingerprint(), gamma.cache_fingerprint());
        let parsed: RenderContract =
            serde_json::from_value(serde_json::to_value(&gamma).unwrap()).unwrap();
        assert_eq!(parsed, gamma);
        let legacy: RenderContract = serde_json::from_value(json).unwrap();
        assert_eq!(legacy.text_gamma, TextGamma::Linear);
    }

    #[test]
    fn forced_colors_change_identity_only_when_set() {
        let base = contract(1);
        let json = serde_json::to_value(&base).expect("serialize contract");
        assert!(
            json.get("forced_colors").is_none(),
            "unset forced colours keep the legacy serialized identity"
        );
        let mut forced = base.clone();
        forced.forced_colors = Some(ForcedColorScheme::uniform(ContractColor::rgba(
            255, 255, 0, 255,
        )));
        assert_ne!(base.cache_fingerprint(), forced.cache_fingerprint());
        let mut stroke = forced.clone();
        stroke
            .forced_colors
            .as_mut()
            .unwrap()
            .convert_fill_to_stroke = true;
        assert_ne!(forced.cache_fingerprint(), stroke.cache_fingerprint());
        let parsed: RenderContract =
            serde_json::from_value(serde_json::to_value(&stroke).unwrap()).unwrap();
        assert_eq!(parsed, stroke);
        let legacy: RenderContract = serde_json::from_value(json).unwrap();
        assert_eq!(legacy.forced_colors, None);
    }

    #[test]
    fn field_effect_registry_covers_every_serialized_contract_field() {
        let mut contract = contract(1);
        // The form highlight is only serialized while it is switched on.
        contract.form_highlight.focus_field = Some("name".to_string());
        contract.forced_colors = Some(ForcedColorScheme::uniform(ContractColor::default()));
        contract.xfa = XfaRenderPolicy::Foreground;
        contract.text_gamma = TextGamma::Pdfium;
        let serialized = serde_json::to_value(&contract).expect("serialize render contract");
        let serialized_fields = serialized
            .as_object()
            .expect("render contract serializes as an object")
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        let effect_fields = render_contract_field_effects()
            .iter()
            .map(|effect| effect.field.to_string())
            .collect::<std::collections::BTreeSet<_>>();

        assert_eq!(serialized_fields, effect_fields);
        assert!(render_contract_field_effects()
            .iter()
            .all(|effect| effect.cache_identity && effect.active_execution));
    }
}
