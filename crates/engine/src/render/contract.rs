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
        effect: "standard-or-research-policy-identity",
    },
    RenderContractFieldEffect {
        field: "backend",
        cache_identity: true,
        validation: false,
        active_execution: true,
        binding_builder: true,
        effect: "scalar-standard-or-research-hybrid-dispatch",
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
    fn field_effect_registry_covers_every_serialized_contract_field() {
        let contract = contract(1);
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
