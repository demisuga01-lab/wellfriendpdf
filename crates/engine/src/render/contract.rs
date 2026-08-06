//! Versioned, binding-safe render contract.
//!
//! This module keeps every public pixel-affecting choice in one immutable,
//! serializable value. Callers that need an option not implemented by the
//! selected backend receive a typed error instead of silently getting a
//! different rendering policy.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Result, WellfriendError};

use super::buffer::RenderMode;
use super::display_list::RenderTile;
use super::transform::Viewport;

pub const RENDER_CONTRACT_SCHEMA_VERSION: u32 = 1;

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RevisionId(pub u64);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ObjectIdentityId(pub u32);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ResourceId(pub u32);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SourceLinkId(pub u32);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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
}

impl PixelFormat {
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Rgba8 | Self::Bgra8 => 4,
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExecutionMode {
    #[default]
    Standard,
    Research,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BackendSelection {
    ScalarReference,
    #[default]
    StandardCpu,
    ResearchHybrid,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SmoothingPolicy {
    Disabled,
    #[default]
    Antialiased,
    Subpixel,
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ColorScheme {
    #[default]
    Light,
    Dark,
    ForcedMonochrome,
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
    BestEffortResearch,
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
    pub optional_content: OptionalContentStateId,
    pub text_smoothing: SmoothingPolicy,
    pub image_smoothing: SmoothingPolicy,
    pub path_smoothing: SmoothingPolicy,
    pub subpixel_text: SmoothingPolicy,
    pub grayscale: bool,
    pub color_scheme: ColorScheme,
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
            optional_content: OptionalContentStateId::default(),
            text_smoothing: SmoothingPolicy::Antialiased,
            image_smoothing: SmoothingPolicy::Antialiased,
            path_smoothing: SmoothingPolicy::Antialiased,
            subpixel_text: SmoothingPolicy::Disabled,
            grayscale: false,
            color_scheme: ColorScheme::Light,
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
        if let Some(clip) = self.clip {
            if clip.width == 0 || clip.height == 0 {
                return Err(WellfriendError::invalid_input(
                    "render contract clip must have non-zero dimensions",
                ));
            }
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
    fn defaults_are_deterministic_and_valid() {
        let contract = contract(7);
        contract.validate().expect("default contract is valid");
        assert_eq!(contract.render_mode(), RenderMode::Compat);
        assert_eq!(contract.cache_fingerprint(), contract.cache_fingerprint());
    }
}
