//! Image decode planning: metadata-first culling and complete cache identity.
//!
//! This module implements the technically actionable portion of RB-06/RB-13:
//! metadata-first image culling and cache-identity completeness for the active
//! renderer. Before decoding any image XObject, the renderer inspects image
//! metadata (dimensions, object identity) and the conservative device bounds
//! (computed from the current CTM and the unit-square image domain) to determine
//! whether the decoded pixels would intersect the active tile/viewport. Images
//! that are entirely outside the viewport are skipped without invoking the
//! decode pipeline or occupying cache slots.
//!
//! Additionally, the image cache key is extended with fields that are relevant
//! for source-region selection, reduction level selection, output-affecting
//! image dictionary state, and render-contract state. These fields ensure that
//! a cache entry produced for one viewport/transform/reduction state is never
//! incorrectly reused when the decode contract changes. Active renderer planning
//! requests native source windows for guarded CCITT/raw paths and native reduced
//! output for guarded JPEG/JPX paths, while other codec shapes remain explicit
//! full-output plans.
//!
//! # Upstream decoder limitations (as of this implementation)
//!
//! - **JPEG (via `jpeg-decoder`)**: No region decode or progressive public
//!   continuation API. Native reduced-IDCT decode is exposed for 1/2, 1/4, and
//!   1/8 output tiers, and guarded DCT image plans use it when no full-image
//!   mask/postprocessing step needs the original sample grid.
//!
//! - **JPEG 2000 / JPX (via `hayro-jpeg2000`)**: the pure-Rust adapter used by
//!   the renderer exposes a target-resolution decode hint, so guarded
//!   downscale plans use native reduced-resolution output when no full-image
//!   mask/postprocessing step needs the original sample grid. Region-of-interest,
//!   codestream tile decode, and incremental tile-part continuation APIs remain
//!   unavailable.
//!
//! - **JBIG2 (via `jbig2dec` FFI or fallback)**: No region decode API. The
//!   segment model theoretically supports stripe-based decode, but available
//!   Rust wrappers do not expose partial decoding.
//!
//! - **CCITT Fax (internal)**: Axis-aligned, source-clipped grayscale image
//!   XObjects and inline images can use the bounded source-window sink when no
//!   reduced-resolution decode or full-image postprocessing is required.
//!
//! - **Raw unfiltered samples (internal)**: Axis-aligned, source-clipped,
//!   1/2/4/8/16-bit image XObjects and inline images can use a bounded raw
//!   source-window crop when no reduction or full-image postprocessing is
//!   required.
//!
//! - **Flate/LZW/ASCII/RunLength (internal)**: These are generic stream
//!   filters, not image codecs. No spatial awareness. Full decompression is
//!   required before predictor application.
//!
//! The `source_region` and `reduction_level` fields in [`ImageDecodePlan`] and
//! [`ImageDecodeCacheKey`] are populated for the guarded CCITT/raw
//! source-window paths plus JPEG/JPX native reduced-resolution paths. Other
//! codec shapes stay conservative. When a future decoder or planner upgrade
//! exposes broader partial APIs, these fields will drive actual partial decode
//! without changing cache semantics.
//!
//! The same decision is also surfaced as a typed
//! [`ImageDecodeCapabilityReport`]. This keeps JPX/JBIG2/JPEG/lossless behavior
//! explicit at the source boundary: callers can distinguish "full decode only"
//! from a future native region/reduction implementation without scraping cache
//! keys or inferring from filter names. The module also exposes a bounded
//! progressive-image-decode session state machine. Current codec adapters do not
//! fake native continuation: full-output paths report typed
//! `full_decode_required` outcomes, while bounded native region/reduction plans
//! complete as planned partial decode work behind the same lifecycle.

use crate::images::locator::ImageReference;
use crate::render::contract::BackendSelection;
use crate::render::display_list::RenderBounds;
use crate::render::transform::{Transform2D, Viewport};

// ---------------------------------------------------------------------------
// Image metadata extracted from the dictionary before decode.
// ---------------------------------------------------------------------------

/// Lightweight image metadata extracted from the PDF image XObject dictionary
/// without triggering any stream decompression or pixel decode.
#[derive(Debug, Clone)]
pub(crate) struct ImageMetadata {
    /// PDF object number (0 for inline images).
    pub object_number: u32,
    /// PDF generation number.
    pub generation_number: u16,
    /// Image width in samples.
    pub width: u32,
    /// Image height in samples.
    pub height: u32,
    /// Bits per component (1..16).
    pub bits_per_component: u8,
    /// Canonical color space family name.
    pub color_space: String,
    /// Filter chain names.
    pub filters: Vec<String>,
    /// Whether the image is a stencil mask.
    pub is_mask: bool,
    /// Whether this is an inline image.
    pub is_inline: bool,
    /// Whether later processing needs the complete decoded image, e.g. soft
    /// masks or explicit image masks that are not cropped alongside this image.
    pub requires_full_image_postprocessing: bool,
    /// Stable fingerprint of the image `/Decode` array, or `none`.
    pub decode_fingerprint: String,
    /// Stable fingerprint of `/DecodeParms` semantics, or `none`.
    pub decode_params_fingerprint: String,
    /// Stable fingerprint of an explicit `/Mask` entry or image-mask mode.
    pub image_mask_fingerprint: String,
    /// Stable fingerprint of an explicit `/SMask` entry, or `none`.
    pub soft_mask_fingerprint: String,
    /// Whether image interpolation is requested by the PDF image dictionary.
    pub interpolate: bool,
    /// Decoder component selection for this image.
    pub component_selection: ImageComponentSelection,
}

// ---------------------------------------------------------------------------
// Source region / reduction / contract fields for cache identity.
// ---------------------------------------------------------------------------

/// Describes the source region of interest for an image decode operation.
///
/// Most upstream decoders still require full-image decode, but guarded CCITT
/// XObject and inline-image paths can request a bounded source sub-rectangle.
/// The value is part of cache identity so full and partial outputs cannot
/// collide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
pub enum ImageSourceRegion {
    /// Full image decode.
    Full,
    /// A sub-rectangle in image-sample coordinates.
    SubRect {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
}

/// Resolution reduction tier for the image decode.
///
/// Native JPEG/JPX reduction plans use this when the reduced output covers
/// the requested target dimensions; other paths keep full resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
pub enum ImageReductionLevel {
    /// Full resolution decode.
    None,
    /// Power-of-two reduction (e.g., 1 = half, 2 = quarter).
    PowerOfTwo(u8),
}

/// Component subset requested from the image decoder.
///
/// Current codec adapters decode every output component, but JPX and future
/// native decoders can expose component-selective paths. Keeping this in cache
/// identity prevents all-component and subset decodes from sharing pixels.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize)]
pub enum ImageComponentSelection {
    /// Decode every component needed by the resolved image color model.
    All,
    /// Decode a stable subset of zero-based component indexes.
    Components(Vec<u8>),
}

impl ImageSourceRegion {
    fn cache_fragment(self) -> String {
        match self {
            Self::Full => "full".to_string(),
            Self::SubRect {
                x,
                y,
                width,
                height,
            } => format!("subrect:{x}:{y}:{width}:{height}"),
        }
    }
}

impl ImageReductionLevel {
    fn cache_fragment(self) -> String {
        match self {
            Self::None => "none".to_string(),
            Self::PowerOfTwo(level) => format!("pow2:{level}"),
        }
    }
}

impl ImageComponentSelection {
    fn cache_fragment(&self) -> String {
        match self {
            Self::All => "all".to_string(),
            Self::Components(components) => {
                let mut out = String::from("components");
                for component in components {
                    out.push(':');
                    out.push_str(&component.to_string());
                }
                out
            }
        }
    }
}

/// Normalized decoded output shape stored in the image decode cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
pub(crate) enum ImageDecodeTargetFormat {
    /// Current renderer cache output: 8-bit interleaved `RawImage` channels.
    RawImage8Interleaved,
}

impl ImageDecodeTargetFormat {
    pub(crate) fn for_backend(backend: BackendSelection) -> Self {
        match backend {
            BackendSelection::ScalarReference
            | BackendSelection::StandardCpu
            | BackendSelection::ResearchHybrid => Self::RawImage8Interleaved,
        }
    }

    fn cache_fragment(self) -> &'static str {
        match self {
            Self::RawImage8Interleaved => "raw-image-8-interleaved",
        }
    }
}

/// Renderer backend identity for image decode cache separation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
pub(crate) enum ImageDecodeBackendIdentity {
    /// Active CPU renderer path selected by the current contract backend.
    StandardCpu,
    /// Active scalar-reference contract path, sharing raw output but not cache identity.
    ScalarReference,
    /// Reserved for research/hybrid backends once they own distinct decode output.
    ResearchHybrid,
}

impl ImageDecodeBackendIdentity {
    fn cache_fragment(self) -> &'static str {
        match self {
            Self::StandardCpu => "standard-cpu",
            Self::ScalarReference => "scalar-reference",
            Self::ResearchHybrid => "research-hybrid",
        }
    }
}

impl From<BackendSelection> for ImageDecodeBackendIdentity {
    fn from(value: BackendSelection) -> Self {
        match value {
            BackendSelection::ScalarReference => Self::ScalarReference,
            BackendSelection::StandardCpu => Self::StandardCpu,
            BackendSelection::ResearchHybrid => Self::ResearchHybrid,
        }
    }
}

/// Output and contract identity for one image decode cache entry.
#[derive(Debug, Clone)]
pub(crate) struct ImageDecodePlanIdentity {
    pub target_format: ImageDecodeTargetFormat,
    pub backend: ImageDecodeBackendIdentity,
    pub render_contract_fingerprint: String,
}

impl ImageDecodePlanIdentity {
    pub fn for_backend(
        backend: BackendSelection,
        render_contract_fingerprint: impl Into<String>,
    ) -> Self {
        Self {
            target_format: ImageDecodeTargetFormat::for_backend(backend),
            backend: ImageDecodeBackendIdentity::from(backend),
            render_contract_fingerprint: render_contract_fingerprint.into(),
        }
    }
}

/// Codec family selected by the image filter chain for decode planning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
pub enum ImageDecodeCodec {
    /// Unfiltered raw image samples.
    Raw,
    /// Raw samples or generic lossless stream filters handled as full images.
    Lossless,
    /// PDF `/DCTDecode` JPEG image data.
    Jpeg,
    /// PDF `/JPXDecode` JPEG 2000 image data.
    Jpx,
    /// PDF `/JBIG2Decode` image data.
    Jbig2,
    /// PDF `/CCITTFaxDecode` image data.
    Ccitt,
    /// A filter chain the planner cannot classify.
    Unknown,
}

impl ImageDecodeCodec {
    fn cache_fragment(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Lossless => "lossless",
            Self::Jpeg => "jpeg",
            Self::Jpx => "jpx",
            Self::Jbig2 => "jbig2",
            Self::Ccitt => "ccitt",
            Self::Unknown => "unknown",
        }
    }
}

/// Reason a native partial-decode capability is unavailable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
pub enum ImageDecodeUnavailableReason {
    /// Generic stream/predictor processing is not spatially aware.
    GenericStreamFilterFullMaterialization,
    /// The linked decoder does not expose this operation through its Rust API.
    DecoderApiUnavailable,
    /// The internal decoder has a windowed path, but active planning still requests full output.
    InternalWindowedDecodeNotIntegrated,
    /// The internal raw decoder has a windowed path, but this image shape is outside it.
    RawWindowUnsupportedShape,
    /// The terminal monochrome image decoder cannot satisfy the declared image shape.
    MonochromeTerminalShapeUnsupported,
    /// The active decoder cannot return a requested component subset natively.
    ComponentSelectionUnavailable,
    /// The active decoder cannot return JPEG 2000 codestream tiles natively.
    CodestreamTileDecodeUnavailable,
    /// The planner could not classify the filter chain.
    UnknownCodec,
}

/// Availability of one native decode capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
pub enum ImageDecodeCapabilityStatus {
    /// A native codec path can satisfy the requested capability.
    Native,
    /// The active implementation requires full-image decode for this capability.
    Unavailable(ImageDecodeUnavailableReason),
}

impl ImageDecodeCapabilityStatus {
    /// True when this capability is not available natively.
    pub fn is_unavailable(self) -> bool {
        matches!(self, Self::Unavailable(_))
    }
}

/// Where decode execution controls are enforced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageDecodeExecutionControlStatus {
    /// The selected codec API can enforce the control during native decode.
    NativeCodec,
    /// The renderer enforces the control at scheduler, adapter, or finalizer boundaries.
    RendererBoundary,
    /// The control is unavailable for this codec/path.
    Unavailable(ImageDecodeUnavailableReason),
}

impl ImageDecodeExecutionControlStatus {
    pub fn is_native_codec(self) -> bool {
        matches!(self, Self::NativeCodec)
    }

    pub fn is_renderer_boundary(self) -> bool {
        matches!(self, Self::RendererBoundary)
    }
}

/// Explicit source-level capability report for an image decode plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
pub struct ImageDecodeCapabilityReport {
    /// Codec family chosen from the image filter chain.
    pub codec: ImageDecodeCodec,
    /// Source region the active decoder will request.
    pub source_region: ImageSourceRegion,
    /// Resolution level the active decoder will request.
    pub reduction_level: ImageReductionLevel,
    /// Native metadata/header inspection support before pixel decode.
    pub metadata_inspection: ImageDecodeCapabilityStatus,
    /// Native source-region decode support.
    pub region_decode: ImageDecodeCapabilityStatus,
    /// Native resolution-reduction decode support.
    pub reduction_decode: ImageDecodeCapabilityStatus,
    /// Native progressive/incremental image decode support.
    pub progressive_decode: ImageDecodeCapabilityStatus,
    /// Native codec tile decode support, distinct from renderer output tiles.
    pub tile_decode: ImageDecodeCapabilityStatus,
    /// Native component-subset decode support.
    pub component_decode: ImageDecodeCapabilityStatus,
    /// Cancellation enforcement available to this decode path.
    pub cancellation: ImageDecodeExecutionControlStatus,
    /// Memory-budget enforcement available to this decode path.
    pub memory_budget: ImageDecodeExecutionControlStatus,
}

/// Lifecycle state for an image decode work item inside progressive rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressiveImageDecodeState {
    Created,
    Started,
    Paused,
    Completed,
    Cancelled,
    Failed,
    Closed,
}

/// Why a progressive image decode session released its retained decoder state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressiveImageDecodeReleaseReason {
    Cancellation,
    RenderFailure,
    SessionClose,
    DocumentClose,
}

/// A bounded progressive decode request derived from [`ImageDecodePlan`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize)]
pub struct ProgressiveImageDecodeRequest {
    pub request_id: String,
    pub cache_key: String,
    pub capability_report: ImageDecodeCapabilityReport,
    pub requires_component_decode: bool,
    pub max_retained_bytes: usize,
}

/// Report returned by the progressive image decode lifecycle.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ProgressiveImageDecodeReport {
    pub request_id: String,
    pub state: ProgressiveImageDecodeState,
    pub phase: &'static str,
    pub codec: ImageDecodeCodec,
    pub source_region: ImageSourceRegion,
    pub reduction_level: ImageReductionLevel,
    pub retained_bytes: usize,
    pub completed: bool,
    pub resumable: bool,
    pub native_progressive_available: bool,
    pub requires_component_decode: bool,
    pub full_decode_required: bool,
    pub unavailable_reason: Option<ImageDecodeUnavailableReason>,
    pub release_reason: Option<ProgressiveImageDecodeReleaseReason>,
}

/// Source-level progressive image decode lifecycle.
///
/// It owns no decoded pixels today because active codec adapters do not expose
/// resumable progressive continuation. The object is still useful to progressive
/// renderers and bindings: it gives start/continue/pause/resume/cancel/fail/close
/// semantics, bounded retained state, and a precise typed report that says
/// whether native continuation is available.
#[derive(Debug, Clone)]
pub struct ProgressiveImageDecodeSession {
    request: ProgressiveImageDecodeRequest,
    state: ProgressiveImageDecodeState,
    retained_bytes: usize,
    release_reason: Option<ProgressiveImageDecodeReleaseReason>,
}

impl ImageDecodeCapabilityReport {
    fn full_decode_only(codec: ImageDecodeCodec, reason: ImageDecodeUnavailableReason) -> Self {
        Self {
            codec,
            source_region: ImageSourceRegion::Full,
            reduction_level: ImageReductionLevel::None,
            metadata_inspection: ImageDecodeCapabilityStatus::Unavailable(reason),
            region_decode: ImageDecodeCapabilityStatus::Unavailable(reason),
            reduction_decode: ImageDecodeCapabilityStatus::Unavailable(reason),
            progressive_decode: ImageDecodeCapabilityStatus::Unavailable(reason),
            tile_decode: ImageDecodeCapabilityStatus::Unavailable(reason),
            component_decode: ImageDecodeCapabilityStatus::Unavailable(reason),
            cancellation: ImageDecodeExecutionControlStatus::RendererBoundary,
            memory_budget: ImageDecodeExecutionControlStatus::RendererBoundary,
        }
    }

    fn jpeg(
        reduction_level: ImageReductionLevel,
        reduction_decode: ImageDecodeCapabilityStatus,
    ) -> Self {
        Self {
            codec: ImageDecodeCodec::Jpeg,
            source_region: ImageSourceRegion::Full,
            reduction_level,
            metadata_inspection: ImageDecodeCapabilityStatus::Native,
            region_decode: ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::DecoderApiUnavailable,
            ),
            reduction_decode,
            progressive_decode: ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::DecoderApiUnavailable,
            ),
            tile_decode: ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::CodestreamTileDecodeUnavailable,
            ),
            component_decode: ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::ComponentSelectionUnavailable,
            ),
            cancellation: ImageDecodeExecutionControlStatus::RendererBoundary,
            memory_budget: ImageDecodeExecutionControlStatus::RendererBoundary,
        }
    }

    fn jpx(
        reduction_level: ImageReductionLevel,
        reduction_decode: ImageDecodeCapabilityStatus,
    ) -> Self {
        Self {
            codec: ImageDecodeCodec::Jpx,
            source_region: ImageSourceRegion::Full,
            reduction_level,
            metadata_inspection: ImageDecodeCapabilityStatus::Native,
            region_decode: ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::DecoderApiUnavailable,
            ),
            reduction_decode,
            progressive_decode: ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::DecoderApiUnavailable,
            ),
            tile_decode: ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::CodestreamTileDecodeUnavailable,
            ),
            component_decode: ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::ComponentSelectionUnavailable,
            ),
            cancellation: ImageDecodeExecutionControlStatus::RendererBoundary,
            memory_budget: ImageDecodeExecutionControlStatus::RendererBoundary,
        }
    }

    fn ccitt(
        source_region: ImageSourceRegion,
        region_decode: ImageDecodeCapabilityStatus,
        component_decode: ImageDecodeCapabilityStatus,
    ) -> Self {
        Self {
            codec: ImageDecodeCodec::Ccitt,
            source_region,
            reduction_level: ImageReductionLevel::None,
            metadata_inspection: ImageDecodeCapabilityStatus::Native,
            region_decode,
            reduction_decode: ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::DecoderApiUnavailable,
            ),
            progressive_decode: ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::DecoderApiUnavailable,
            ),
            tile_decode: ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::DecoderApiUnavailable,
            ),
            component_decode,
            cancellation: ImageDecodeExecutionControlStatus::RendererBoundary,
            memory_budget: ImageDecodeExecutionControlStatus::RendererBoundary,
        }
    }

    fn raw(
        source_region: ImageSourceRegion,
        region_decode: ImageDecodeCapabilityStatus,
        component_decode: ImageDecodeCapabilityStatus,
    ) -> Self {
        Self {
            codec: ImageDecodeCodec::Raw,
            source_region,
            reduction_level: ImageReductionLevel::None,
            metadata_inspection: ImageDecodeCapabilityStatus::Native,
            region_decode,
            reduction_decode: ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::DecoderApiUnavailable,
            ),
            progressive_decode: ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::DecoderApiUnavailable,
            ),
            tile_decode: ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::DecoderApiUnavailable,
            ),
            component_decode,
            cancellation: ImageDecodeExecutionControlStatus::RendererBoundary,
            memory_budget: ImageDecodeExecutionControlStatus::RendererBoundary,
        }
    }

    /// True when the plan must decode the whole image at full resolution.
    pub fn requires_full_decode(self) -> bool {
        self.source_region == ImageSourceRegion::Full
            && self.reduction_level == ImageReductionLevel::None
    }

    /// True when the codec exposes no native spatial, reduction, or component
    /// path that can avoid a full-image decode-only strategy.
    pub fn is_full_decode_only(self) -> bool {
        self.requires_full_decode()
            && self.region_decode.is_unavailable()
            && self.reduction_decode.is_unavailable()
            && self.component_decode.is_unavailable()
    }

    pub fn progressive_unavailable_reason(self) -> Option<ImageDecodeUnavailableReason> {
        match self.progressive_decode {
            ImageDecodeCapabilityStatus::Native => None,
            ImageDecodeCapabilityStatus::Unavailable(reason) => Some(reason),
        }
    }

    /// True when the active non-progressive decoder plan is already bounded by
    /// a native source-window or native reduced-resolution request. These paths
    /// do not expose resumable codec continuation, but progressive renderers can
    /// complete the image work item without escalating it to a full-image decode.
    pub fn uses_native_bounded_decode(self) -> bool {
        let native_region = matches!(self.source_region, ImageSourceRegion::SubRect { .. })
            && matches!(self.region_decode, ImageDecodeCapabilityStatus::Native);
        let native_reduction = self.reduction_level != ImageReductionLevel::None
            && matches!(self.reduction_decode, ImageDecodeCapabilityStatus::Native);
        native_region || native_reduction
    }
}

impl ProgressiveImageDecodeRequest {
    pub fn new(
        cache_key: impl Into<String>,
        capability_report: ImageDecodeCapabilityReport,
        max_retained_bytes: usize,
    ) -> Self {
        let cache_key = cache_key.into();
        Self {
            request_id: format!("image-decode:{:016x}", stable_hash64(cache_key.as_bytes())),
            cache_key,
            capability_report,
            requires_component_decode: false,
            max_retained_bytes,
        }
    }

    pub fn with_component_decode_requirement(mut self, requires_component_decode: bool) -> Self {
        self.requires_component_decode = requires_component_decode;
        self
    }

    pub fn uses_native_bounded_decode(&self) -> bool {
        self.capability_report.uses_native_bounded_decode()
            || (self.requires_component_decode
                && matches!(
                    self.capability_report.component_decode,
                    ImageDecodeCapabilityStatus::Native
                ))
    }

    #[cfg(test)]
    pub(crate) fn from_plan(plan: &ImageDecodePlan, max_retained_bytes: usize) -> Self {
        Self::new(
            plan.cache_key.to_cache_string(),
            plan.capability_report,
            max_retained_bytes,
        )
        .with_component_decode_requirement(plan.requires_component_decode)
    }
}

impl ProgressiveImageDecodeSession {
    pub fn new(request: ProgressiveImageDecodeRequest) -> Self {
        Self {
            request,
            state: ProgressiveImageDecodeState::Created,
            retained_bytes: 0,
            release_reason: None,
        }
    }

    pub fn start(&mut self) -> ProgressiveImageDecodeReport {
        if self.state == ProgressiveImageDecodeState::Created {
            self.state = ProgressiveImageDecodeState::Started;
        }
        self.report("start")
    }

    pub fn continue_decode(&mut self) -> ProgressiveImageDecodeReport {
        if matches!(
            self.state,
            ProgressiveImageDecodeState::Completed
                | ProgressiveImageDecodeState::Cancelled
                | ProgressiveImageDecodeState::Failed
                | ProgressiveImageDecodeState::Closed
        ) {
            return self.report("continue_terminal");
        }

        match self.state {
            ProgressiveImageDecodeState::Created => {
                self.state = ProgressiveImageDecodeState::Started;
            }
            ProgressiveImageDecodeState::Paused => {
                self.state = ProgressiveImageDecodeState::Started;
            }
            ProgressiveImageDecodeState::Started => {}
            ProgressiveImageDecodeState::Completed
            | ProgressiveImageDecodeState::Cancelled
            | ProgressiveImageDecodeState::Failed
            | ProgressiveImageDecodeState::Closed => unreachable!("terminal states returned above"),
        }

        if matches!(
            self.request.capability_report.progressive_decode,
            ImageDecodeCapabilityStatus::Native
        ) {
            self.state = ProgressiveImageDecodeState::Completed;
            self.report("native_progressive_complete")
        } else if self.request.uses_native_bounded_decode() {
            self.retained_bytes = 0;
            self.state = ProgressiveImageDecodeState::Completed;
            self.report("planned_partial_decode_complete")
        } else {
            self.report("full_decode_required")
        }
    }

    pub fn pause(&mut self) -> ProgressiveImageDecodeReport {
        if self.state == ProgressiveImageDecodeState::Started {
            self.state = ProgressiveImageDecodeState::Paused;
        }
        self.report("pause")
    }

    pub fn resume(&mut self) -> ProgressiveImageDecodeReport {
        if self.state == ProgressiveImageDecodeState::Paused {
            self.state = ProgressiveImageDecodeState::Started;
        }
        self.report("resume")
    }

    pub fn cancel(&mut self) -> ProgressiveImageDecodeReport {
        self.release_decoder_state(
            ProgressiveImageDecodeState::Cancelled,
            ProgressiveImageDecodeReleaseReason::Cancellation,
        );
        self.report("cancel")
    }

    pub fn fail(&mut self) -> ProgressiveImageDecodeReport {
        self.release_decoder_state(
            ProgressiveImageDecodeState::Failed,
            ProgressiveImageDecodeReleaseReason::RenderFailure,
        );
        self.report("fail")
    }

    pub fn close(&mut self) -> ProgressiveImageDecodeReport {
        self.release_decoder_state(
            ProgressiveImageDecodeState::Closed,
            ProgressiveImageDecodeReleaseReason::SessionClose,
        );
        self.report("close")
    }

    pub fn close_for_document_close(&mut self) -> ProgressiveImageDecodeReport {
        self.release_decoder_state(
            ProgressiveImageDecodeState::Closed,
            ProgressiveImageDecodeReleaseReason::DocumentClose,
        );
        self.report("document_close")
    }

    fn release_decoder_state(
        &mut self,
        terminal_state: ProgressiveImageDecodeState,
        reason: ProgressiveImageDecodeReleaseReason,
    ) {
        self.retained_bytes = 0;
        self.state = terminal_state;
        self.release_reason = Some(reason);
    }

    pub fn report(&self, phase: &'static str) -> ProgressiveImageDecodeReport {
        let native_progressive_available = matches!(
            self.request.capability_report.progressive_decode,
            ImageDecodeCapabilityStatus::Native
        );
        let terminal = matches!(
            self.state,
            ProgressiveImageDecodeState::Completed
                | ProgressiveImageDecodeState::Cancelled
                | ProgressiveImageDecodeState::Failed
                | ProgressiveImageDecodeState::Closed
        );
        let native_bounded_decode_available = self.request.uses_native_bounded_decode();
        ProgressiveImageDecodeReport {
            request_id: self.request.request_id.clone(),
            state: self.state,
            phase,
            codec: self.request.capability_report.codec,
            source_region: self.request.capability_report.source_region,
            reduction_level: self.request.capability_report.reduction_level,
            retained_bytes: self.retained_bytes.min(self.request.max_retained_bytes),
            completed: self.state == ProgressiveImageDecodeState::Completed,
            resumable: matches!(
                self.state,
                ProgressiveImageDecodeState::Created
                    | ProgressiveImageDecodeState::Started
                    | ProgressiveImageDecodeState::Paused
            ),
            native_progressive_available,
            requires_component_decode: self.request.requires_component_decode,
            full_decode_required: !terminal
                && !native_progressive_available
                && !native_bounded_decode_available
                && self.request.capability_report.requires_full_decode(),
            unavailable_reason: self
                .request
                .capability_report
                .progressive_unavailable_reason(),
            release_reason: self.release_reason,
        }
    }
}

fn stable_hash64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Classify image decode capabilities from the PDF image filter chain.
pub fn image_decode_capabilities_for_filters(filters: &[String]) -> ImageDecodeCapabilityReport {
    let codec = if let Some(last_filter) = filters.last() {
        let Some(codec) = image_decode_codec_for_filter(last_filter) else {
            return ImageDecodeCapabilityReport::full_decode_only(
                ImageDecodeCodec::Unknown,
                ImageDecodeUnavailableReason::UnknownCodec,
            );
        };
        if filters[..filters.len() - 1]
            .iter()
            .any(|filter| image_decode_codec_for_filter(filter).is_none())
        {
            return ImageDecodeCapabilityReport::full_decode_only(
                ImageDecodeCodec::Unknown,
                ImageDecodeUnavailableReason::UnknownCodec,
            );
        }
        codec
    } else {
        ImageDecodeCodec::Raw
    };

    match codec {
        ImageDecodeCodec::Raw => ImageDecodeCapabilityReport::raw(
            ImageSourceRegion::Full,
            ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::InternalWindowedDecodeNotIntegrated,
            ),
            ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::ComponentSelectionUnavailable,
            ),
        ),
        ImageDecodeCodec::Lossless => ImageDecodeCapabilityReport::full_decode_only(
            codec,
            ImageDecodeUnavailableReason::GenericStreamFilterFullMaterialization,
        ),
        ImageDecodeCodec::Jpeg => ImageDecodeCapabilityReport::jpeg(
            ImageReductionLevel::None,
            ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::DecoderApiUnavailable,
            ),
        ),
        ImageDecodeCodec::Jpx => ImageDecodeCapabilityReport::jpx(
            ImageReductionLevel::None,
            ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::DecoderApiUnavailable,
            ),
        ),
        ImageDecodeCodec::Jbig2 => ImageDecodeCapabilityReport::full_decode_only(
            codec,
            ImageDecodeUnavailableReason::DecoderApiUnavailable,
        ),
        ImageDecodeCodec::Ccitt => ImageDecodeCapabilityReport::ccitt(
            ImageSourceRegion::Full,
            ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::InternalWindowedDecodeNotIntegrated,
            ),
            ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::ComponentSelectionUnavailable,
            ),
        ),
        ImageDecodeCodec::Unknown => ImageDecodeCapabilityReport::full_decode_only(
            codec,
            ImageDecodeUnavailableReason::UnknownCodec,
        ),
    }
}

fn image_decode_codec_for_filter(filter: &str) -> Option<ImageDecodeCodec> {
    match filter {
        "DCTDecode" | "DCT" => Some(ImageDecodeCodec::Jpeg),
        "JPXDecode" | "JPX" => Some(ImageDecodeCodec::Jpx),
        "JBIG2Decode" => Some(ImageDecodeCodec::Jbig2),
        "CCITTFaxDecode" | "CCF" => Some(ImageDecodeCodec::Ccitt),
        "FlateDecode" | "Fl" | "LZWDecode" | "LZW" | "RunLengthDecode" | "RL"
        | "ASCIIHexDecode" | "AHx" | "ASCII85Decode" | "A85" => Some(ImageDecodeCodec::Lossless),
        _ => None,
    }
}

/// Contract-relevant rendering state that affects how an image should be
/// decoded or post-processed. Changes to these fields mean the decoded image
/// cannot be reused from cache.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ImageContractState {
    /// Active render-contract identity. This is expected to include document
    /// revision and every output-affecting render policy chosen by the caller.
    pub render_contract_fingerprint: String,
    /// Quantized device-space target width (0 when not axis-aligned or unknown).
    pub target_width: u32,
    /// Quantized device-space target height (0 when not axis-aligned or unknown).
    pub target_height: u32,
    /// Whether high-quality interpolation is active.
    pub high_quality: bool,
    /// Codec family selected from the image filter chain.
    pub codec: ImageDecodeCodec,
    /// Source region for this decode operation.
    pub source_region: ImageSourceRegion,
    /// Reduction level for this decode operation.
    pub reduction_level: ImageReductionLevel,
    /// Component subset requested from the decoder.
    pub component_selection: ImageComponentSelection,
    /// Normalized decoded output shape stored in the decode cache.
    pub target_format: ImageDecodeTargetFormat,
    /// Renderer backend identity for decode outputs that are backend-specific.
    pub backend: ImageDecodeBackendIdentity,
    /// Stable fingerprint of decode-array semantics.
    pub decode_fingerprint: String,
    /// Stable fingerprint of filter decode-parameter semantics.
    pub decode_params_fingerprint: String,
    /// Stable fingerprint of explicit image-mask semantics.
    pub image_mask_fingerprint: String,
    /// Stable fingerprint of soft-mask semantics.
    pub soft_mask_fingerprint: String,
    /// Whether image interpolation is active for the paint.
    pub interpolate: bool,
}

// ---------------------------------------------------------------------------
// Complete image decode cache key.
// ---------------------------------------------------------------------------

/// Extended cache key for image XObject decode results.
///
/// This combines the existing object-identity fields with the new
/// source-region, reduction, and contract-relevant state. Two entries with
/// the same object identity but different contract state are cached separately.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ImageDecodeCacheKey {
    /// Base identity: object number, generation, dimensions, BPC, filters.
    pub base_key: String,
    /// Contract state that modifies decode/post-process semantics.
    pub contract: ImageContractState,
}

impl ImageDecodeCacheKey {
    /// Build the complete cache key string. This extends the base key with
    /// contract-relevant fields so that lookups remain string-based (matching
    /// the existing `HashMap<String, Arc<RawImage>>` cache).
    pub fn to_cache_string(&self) -> String {
        format!(
            "{}:contract:{}:dp:{}x{}:{}:codec:{}:sr:{}:rl:{}:comp:{}:fmt:{}:backend:{}:dec:{}:dparms:{}:im:{}:sm:{}:interp:{}",
            self.base_key,
            self.contract.render_contract_fingerprint,
            self.contract.target_width,
            self.contract.target_height,
            if self.contract.high_quality {
                "hq"
            } else {
                "compat"
            },
            self.contract.codec.cache_fragment(),
            self.contract.source_region.cache_fragment(),
            self.contract.reduction_level.cache_fragment(),
            self.contract.component_selection.cache_fragment(),
            self.contract.target_format.cache_fragment(),
            self.contract.backend.cache_fragment(),
            self.contract.decode_fingerprint,
            self.contract.decode_params_fingerprint,
            self.contract.image_mask_fingerprint,
            self.contract.soft_mask_fingerprint,
            self.contract.interpolate,
        )
    }
}

// ---------------------------------------------------------------------------
// Image decode plan.
// ---------------------------------------------------------------------------

/// The decision made by the image decode planner for a particular image XObject.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ImageDecodePlanDecision {
    /// Image is entirely outside the active viewport/tile. Decode is skipped.
    SkipOutsideViewport,
    /// Image intersects the viewport. Full decode should proceed.
    DecodeRequired,
}

/// Complete image decode plan produced by the planner.
#[derive(Debug, Clone)]
pub(crate) struct ImageDecodePlan {
    /// The planning decision.
    pub decision: ImageDecodePlanDecision,
    /// Explicit native decode capability verdict for this image.
    pub capability_report: ImageDecodeCapabilityReport,
    /// True when the active viewport/tile sees only part of the source image.
    pub requires_source_region_decode: bool,
    /// True when the target footprint is smaller than the source sample grid.
    pub requires_reduction_decode: bool,
    /// True when a caller requested only a subset of image components.
    pub requires_component_decode: bool,
    /// Cache key incorporating contract state.
    pub cache_key: ImageDecodeCacheKey,
}

impl ImageDecodePlan {
    pub(crate) fn requires_unavailable_exact_decode_support(&self) -> bool {
        self.decision == ImageDecodePlanDecision::DecodeRequired
            && ((self.requires_source_region_decode
                && self.capability_report.region_decode.is_unavailable())
                || (self.requires_reduction_decode
                    && self.capability_report.reduction_decode.is_unavailable())
                || (self.requires_component_decode
                    && self.capability_report.component_decode.is_unavailable()))
    }

    pub(crate) fn exact_decode_limitation_summary(&self) -> String {
        let mut limitations = Vec::new();
        if self.requires_source_region_decode
            && self.capability_report.region_decode.is_unavailable()
        {
            limitations.push("source-region");
        }
        if self.requires_reduction_decode
            && self.capability_report.reduction_decode.is_unavailable()
        {
            limitations.push("reduced-resolution");
        }
        if self.requires_component_decode
            && self.capability_report.component_decode.is_unavailable()
        {
            limitations.push("component-selection");
        }
        if limitations.is_empty() {
            "none".to_string()
        } else {
            limitations.join("+")
        }
    }
}

// ---------------------------------------------------------------------------
// Planner: compute device bounds and decide whether to decode.
// ---------------------------------------------------------------------------

/// Compute conservative device bounds for an image XObject.
///
/// PDF images occupy the unit square [0,0]–[1,1] in user space, transformed
/// by the current CTM to device space. This function transforms the four
/// corners of that unit square through the CTM and viewport mapping to
/// produce axis-aligned device-pixel bounds.
pub(crate) fn image_device_bounds(ctm: &Transform2D, viewport: &Viewport) -> Option<RenderBounds> {
    RenderBounds::from_unit_square(ctm, viewport, 1.0)
}

/// Compute the device-space target dimensions for axis-aligned images.
/// Returns (width, height) or (0, 0) if the image is not axis-aligned.
pub(crate) fn image_device_target_dimensions(ctm: &Transform2D, viewport: &Viewport) -> (u32, u32) {
    // Check axis-alignment: an axis-aligned CTM has b==0 and c==0.
    if ctm.b.abs() > 1e-10 || ctm.c.abs() > 1e-10 {
        return (0, 0);
    }
    // Compute device extent from unit square
    let combined = ctm.concat(&viewport.to_transform());
    let p0 = combined.transform_point(0.0, 0.0);
    let p1 = combined.transform_point(1.0, 1.0);
    let w = (p1.0 - p0.0).abs();
    let h = (p1.1 - p0.1).abs();
    if !w.is_finite() || !h.is_finite() || w < 1.0 || h < 1.0 {
        return (0, 0);
    }
    (w.round() as u32, h.round() as u32)
}

fn axis_aligned_source_region_for_viewport(
    metadata: &ImageMetadata,
    ctm: &Transform2D,
    viewport: &Viewport,
    device_bounds: &RenderBounds,
) -> Option<ImageSourceRegion> {
    if metadata.width == 0 || metadata.height == 0 {
        return Some(ImageSourceRegion::Full);
    }
    if !ctm.is_axis_aligned() {
        return None;
    }
    let clip_x0 = device_bounds.x0.max(0);
    let clip_y0 = device_bounds.y0.max(0);
    let clip_x1 = device_bounds.x1.min(viewport.width_px as i32);
    let clip_y1 = device_bounds.y1.min(viewport.height_px as i32);
    if clip_x1 <= clip_x0 || clip_y1 <= clip_y0 {
        return None;
    }

    let inverse = ctm.concat(&viewport.to_transform()).inverse()?;
    let points = [
        inverse.transform_point(clip_x0 as f64, clip_y0 as f64),
        inverse.transform_point(clip_x1 as f64, clip_y0 as f64),
        inverse.transform_point(clip_x0 as f64, clip_y1 as f64),
        inverse.transform_point(clip_x1 as f64, clip_y1 as f64),
    ];
    let mut min_u = f64::INFINITY;
    let mut min_v = f64::INFINITY;
    let mut max_u = f64::NEG_INFINITY;
    let mut max_v = f64::NEG_INFINITY;
    for (u, v) in points {
        if !u.is_finite() || !v.is_finite() {
            return None;
        }
        min_u = min_u.min(u);
        min_v = min_v.min(v);
        max_u = max_u.max(u);
        max_v = max_v.max(v);
    }
    if max_u <= 0.0 || max_v <= 0.0 || min_u >= 1.0 || min_v >= 1.0 {
        return None;
    }

    // Keep one source sample of guard band. The paint path may sample just
    // outside the visible viewport edge for nearest/interpolated magnification.
    let margin_u = 1.0 / metadata.width as f64;
    let margin_v = 1.0 / metadata.height as f64;
    min_u = (min_u - margin_u).clamp(0.0, 1.0);
    min_v = (min_v - margin_v).clamp(0.0, 1.0);
    max_u = (max_u + margin_u).clamp(0.0, 1.0);
    max_v = (max_v + margin_v).clamp(0.0, 1.0);
    if max_u <= min_u || max_v <= min_v {
        return None;
    }

    let width = metadata.width as f64;
    let height = metadata.height as f64;
    let x0 = (min_u * width).floor().clamp(0.0, width) as u32;
    let y0 = (min_v * height).floor().clamp(0.0, height) as u32;
    let x1 = (max_u * width).ceil().clamp(0.0, width) as u32;
    let y1 = (max_v * height).ceil().clamp(0.0, height) as u32;
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    if x0 == 0 && y0 == 0 && x1 == metadata.width && y1 == metadata.height {
        Some(ImageSourceRegion::Full)
    } else {
        Some(ImageSourceRegion::SubRect {
            x: x0,
            y: y0,
            width: x1 - x0,
            height: y1 - y0,
        })
    }
}

fn power_of_two_reduction_level_for_target(
    metadata: &ImageMetadata,
    target_width: u32,
    target_height: u32,
) -> ImageReductionLevel {
    if metadata.width == 0
        || metadata.height == 0
        || target_width == 0
        || target_height == 0
        || (target_width >= metadata.width && target_height >= metadata.height)
    {
        return ImageReductionLevel::None;
    }

    fn scaled(len: u32, scale: u32) -> u32 {
        len.saturating_mul(scale).saturating_sub(1) / 8 + 1
    }

    for &(idct_scale, pow2_level) in &[(1, 3), (2, 2), (4, 1)] {
        if scaled(metadata.width, idct_scale) >= target_width
            && scaled(metadata.height, idct_scale) >= target_height
        {
            return ImageReductionLevel::PowerOfTwo(pow2_level);
        }
    }

    ImageReductionLevel::None
}

fn raw_window_supports_bits_per_component(bits_per_component: u8) -> bool {
    matches!(bits_per_component, 1 | 2 | 4 | 8 | 16)
}

fn raw_window_supports_image_shape(metadata: &ImageMetadata) -> bool {
    raw_window_supports_bits_per_component(metadata.bits_per_component)
        && (!metadata.is_mask || metadata.bits_per_component == 1)
}

fn raw_component_decode_supports_bits_per_component(bits_per_component: u8) -> bool {
    raw_window_supports_bits_per_component(bits_per_component)
}

fn raw_component_decode_supports_image_shape(bits_per_component: u8, is_mask: bool) -> bool {
    raw_component_decode_supports_bits_per_component(bits_per_component)
        && (!is_mask || bits_per_component == 1)
}

fn raw_component_decode_status_for_shape(
    bits_per_component: u8,
    is_mask: bool,
) -> ImageDecodeCapabilityStatus {
    if raw_component_decode_supports_image_shape(bits_per_component, is_mask) {
        ImageDecodeCapabilityStatus::Native
    } else {
        ImageDecodeCapabilityStatus::Unavailable(
            ImageDecodeUnavailableReason::ComponentSelectionUnavailable,
        )
    }
}

fn ccitt_component_decode_status_for_shape(
    bits_per_component: u8,
    color_space: &str,
    is_mask: bool,
) -> ImageDecodeCapabilityStatus {
    if bits_per_component == 1 && (is_mask || matches!(color_space, "DeviceGray" | "G")) {
        ImageDecodeCapabilityStatus::Native
    } else {
        ImageDecodeCapabilityStatus::Unavailable(
            ImageDecodeUnavailableReason::ComponentSelectionUnavailable,
        )
    }
}

fn ccitt_window_supports_image_shape(metadata: &ImageMetadata) -> bool {
    metadata.bits_per_component == 1
        && (metadata.is_mask || matches!(metadata.color_space.as_str(), "DeviceGray" | "G"))
}

fn known_source_component_count(metadata: &ImageMetadata) -> Option<u8> {
    if metadata.is_mask {
        return Some(1);
    }
    match metadata.color_space.as_str() {
        "DeviceGray" | "G" | "CalGray" | "Indexed" | "Separation" => Some(1),
        "DeviceRGB" | "RGB" | "CalRGB" | "sRGB" | "Lab" => Some(3),
        "DeviceCMYK" | "CMYK" => Some(4),
        _ => None,
    }
}

fn components_are_canonical_all(components: &[u8], component_count: u8) -> bool {
    components.len() == usize::from(component_count)
        && components
            .iter()
            .copied()
            .enumerate()
            .all(|(index, component)| usize::from(component) == index)
}

fn component_selection_requires_decode(metadata: &ImageMetadata) -> bool {
    let ImageComponentSelection::Components(components) = &metadata.component_selection else {
        return false;
    };
    match known_source_component_count(metadata) {
        Some(component_count) => !components_are_canonical_all(components, component_count),
        None => true,
    }
}

fn component_selection_valid_for_known_count(metadata: &ImageMetadata) -> bool {
    let ImageComponentSelection::Components(components) = &metadata.component_selection else {
        return true;
    };
    let Some(component_count) = known_source_component_count(metadata) else {
        return true;
    };
    !components.is_empty()
        && components
            .iter()
            .copied()
            .all(|component| component < component_count)
        && components.windows(2).all(|pair| pair[0] < pair[1])
}

/// Classify image decode capabilities with image-shape metadata.
///
/// This complements [`image_decode_capabilities_for_filters`], which remains
/// conservative because filter names alone cannot prove raw bit-depth or mask
/// shape support.
pub fn image_decode_capabilities_for_image_reference(
    image: &ImageReference,
) -> ImageDecodeCapabilityReport {
    let report = image_decode_capabilities_for_filters(image.filter.as_slice());
    match report.codec {
        ImageDecodeCodec::Raw => {
            let region_decode = if raw_component_decode_supports_image_shape(
                image.bits_per_component,
                image.is_mask,
            ) {
                ImageDecodeCapabilityStatus::Native
            } else {
                ImageDecodeCapabilityStatus::Unavailable(
                    ImageDecodeUnavailableReason::RawWindowUnsupportedShape,
                )
            };
            ImageDecodeCapabilityReport::raw(
                ImageSourceRegion::Full,
                region_decode,
                raw_component_decode_status_for_shape(image.bits_per_component, image.is_mask),
            )
        }
        ImageDecodeCodec::Ccitt => {
            let metadata = ImageMetadata {
                object_number: image.object_number,
                generation_number: image.generation_number,
                width: image.width,
                height: image.height,
                bits_per_component: image.bits_per_component,
                color_space: image.color_space.clone(),
                filters: image.filter.clone(),
                is_mask: image.is_mask,
                is_inline: image.is_inline,
                requires_full_image_postprocessing: false,
                decode_fingerprint: "unknown".to_string(),
                decode_params_fingerprint: "unknown".to_string(),
                image_mask_fingerprint: "unknown".to_string(),
                soft_mask_fingerprint: "unknown".to_string(),
                interpolate: false,
                component_selection: ImageComponentSelection::All,
            };
            let region_decode = if ccitt_window_supports_image_shape(&metadata) {
                ImageDecodeCapabilityStatus::Native
            } else {
                ImageDecodeCapabilityStatus::Unavailable(
                    ImageDecodeUnavailableReason::MonochromeTerminalShapeUnsupported,
                )
            };
            ImageDecodeCapabilityReport::ccitt(
                ImageSourceRegion::Full,
                region_decode,
                ccitt_component_decode_status_for_shape(
                    image.bits_per_component,
                    image.color_space.as_str(),
                    image.is_mask,
                ),
            )
        }
        _ => report,
    }
}

/// Produce the complete image decode plan for an image XObject or inline image.
///
/// This is the main entry point called from the renderer before invoking
/// `scheduled_decode_image`. If the plan decision is `SkipOutsideViewport`,
/// the caller must not invoke the decoder.
#[cfg(test)]
pub(crate) fn plan_image_decode(
    metadata: &ImageMetadata,
    ctm: &Transform2D,
    viewport: &Viewport,
    base_cache_key: &str,
    high_quality: bool,
) -> ImageDecodePlan {
    let backend = BackendSelection::StandardCpu;
    plan_image_decode_with_identity(
        metadata,
        ctm,
        viewport,
        base_cache_key,
        high_quality,
        ImageDecodePlanIdentity::for_backend(backend, "test-default-render-contract"),
    )
}

/// Produce a decode plan with explicit cache target/backend identities.
pub(crate) fn plan_image_decode_with_identity(
    metadata: &ImageMetadata,
    ctm: &Transform2D,
    viewport: &Viewport,
    base_cache_key: &str,
    high_quality: bool,
    identity: ImageDecodePlanIdentity,
) -> ImageDecodePlan {
    let device_bounds = image_device_bounds(ctm, viewport);
    let intersects = device_bounds
        .as_ref()
        .map(|bounds| {
            bounds.x1 > 0
                && bounds.x0 < viewport.width_px as i32
                && bounds.y1 > 0
                && bounds.y0 < viewport.height_px as i32
        })
        .unwrap_or(true);

    let decision = if intersects {
        ImageDecodePlanDecision::DecodeRequired
    } else {
        ImageDecodePlanDecision::SkipOutsideViewport
    };

    let (target_width, target_height) = image_device_target_dimensions(ctm, viewport);
    let requires_source_region_decode = matches!(decision, ImageDecodePlanDecision::DecodeRequired)
        && device_bounds
            .as_ref()
            .map(|bounds| {
                bounds.x0 < 0
                    || bounds.y0 < 0
                    || bounds.x1 > viewport.width_px as i32
                    || bounds.y1 > viewport.height_px as i32
            })
            .unwrap_or(false);
    let requires_reduction_decode = matches!(decision, ImageDecodePlanDecision::DecodeRequired)
        && target_width > 0
        && target_height > 0
        && (target_width < metadata.width || target_height < metadata.height);
    let requires_component_decode = matches!(decision, ImageDecodePlanDecision::DecodeRequired)
        && component_selection_requires_decode(metadata);
    let mut capability_report = image_decode_capabilities_for_filters(&metadata.filters);
    if capability_report.codec == ImageDecodeCodec::Ccitt {
        let mut source_region = ImageSourceRegion::Full;
        let mut region_decode = ImageDecodeCapabilityStatus::Unavailable(
            ImageDecodeUnavailableReason::InternalWindowedDecodeNotIntegrated,
        );
        let component_decode = if component_selection_valid_for_known_count(metadata) {
            ccitt_component_decode_status_for_shape(
                metadata.bits_per_component,
                metadata.color_space.as_str(),
                metadata.is_mask,
            )
        } else {
            ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::ComponentSelectionUnavailable,
            )
        };
        if !ccitt_window_supports_image_shape(metadata) {
            region_decode = ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::MonochromeTerminalShapeUnsupported,
            );
        } else if !requires_source_region_decode {
            region_decode = ImageDecodeCapabilityStatus::Native;
        } else if !requires_reduction_decode && !metadata.requires_full_image_postprocessing {
            if let Some(planned_region) = device_bounds.as_ref().and_then(|bounds| {
                axis_aligned_source_region_for_viewport(metadata, ctm, viewport, bounds)
            }) {
                source_region = planned_region;
                region_decode = ImageDecodeCapabilityStatus::Native;
            }
        }
        capability_report =
            ImageDecodeCapabilityReport::ccitt(source_region, region_decode, component_decode);
    } else if capability_report.codec == ImageDecodeCodec::Raw {
        let mut source_region = ImageSourceRegion::Full;
        let mut region_decode = ImageDecodeCapabilityStatus::Unavailable(
            ImageDecodeUnavailableReason::InternalWindowedDecodeNotIntegrated,
        );
        let component_decode = if metadata.requires_full_image_postprocessing
            || !component_selection_valid_for_known_count(metadata)
        {
            ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::ComponentSelectionUnavailable,
            )
        } else {
            raw_component_decode_status_for_shape(metadata.bits_per_component, metadata.is_mask)
        };
        if !requires_source_region_decode {
            region_decode = ImageDecodeCapabilityStatus::Native;
        } else if !requires_reduction_decode
            && !metadata.requires_full_image_postprocessing
            && raw_window_supports_image_shape(metadata)
        {
            if let Some(planned_region) = device_bounds.as_ref().and_then(|bounds| {
                axis_aligned_source_region_for_viewport(metadata, ctm, viewport, bounds)
            }) {
                source_region = planned_region;
                region_decode = ImageDecodeCapabilityStatus::Native;
            }
        } else {
            region_decode = ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::RawWindowUnsupportedShape,
            );
        }
        capability_report =
            ImageDecodeCapabilityReport::raw(source_region, region_decode, component_decode);
    } else if matches!(
        capability_report.codec,
        ImageDecodeCodec::Jpeg | ImageDecodeCodec::Jpx
    ) && requires_reduction_decode
        && !metadata.is_mask
        && !metadata.requires_full_image_postprocessing
    {
        let reduction_level =
            power_of_two_reduction_level_for_target(metadata, target_width, target_height);
        if reduction_level != ImageReductionLevel::None {
            capability_report = match capability_report.codec {
                ImageDecodeCodec::Jpeg => ImageDecodeCapabilityReport::jpeg(
                    reduction_level,
                    ImageDecodeCapabilityStatus::Native,
                ),
                ImageDecodeCodec::Jpx => ImageDecodeCapabilityReport::jpx(
                    reduction_level,
                    ImageDecodeCapabilityStatus::Native,
                ),
                _ => capability_report,
            };
        }
    }
    if requires_component_decode && capability_report.codec != ImageDecodeCodec::Raw {
        capability_report.component_decode = ImageDecodeCapabilityStatus::Unavailable(
            ImageDecodeUnavailableReason::ComponentSelectionUnavailable,
        );
    }

    let source_region = capability_report.source_region;
    let reduction_level = capability_report.reduction_level;

    let contract = ImageContractState {
        render_contract_fingerprint: identity.render_contract_fingerprint,
        target_width,
        target_height,
        high_quality,
        codec: capability_report.codec,
        source_region,
        reduction_level,
        component_selection: metadata.component_selection.clone(),
        target_format: identity.target_format,
        backend: identity.backend,
        decode_fingerprint: metadata.decode_fingerprint.clone(),
        decode_params_fingerprint: metadata.decode_params_fingerprint.clone(),
        image_mask_fingerprint: metadata.image_mask_fingerprint.clone(),
        soft_mask_fingerprint: metadata.soft_mask_fingerprint.clone(),
        interpolate: metadata.interpolate,
    };

    let cache_key = ImageDecodeCacheKey {
        base_key: format!(
            "{base_cache_key}:obj:{}:{}:src:{}x{}:{}:cs:{}:mask:{}:inline:{}",
            metadata.object_number,
            metadata.generation_number,
            metadata.width,
            metadata.height,
            metadata.bits_per_component,
            metadata.color_space,
            metadata.is_mask,
            metadata.is_inline,
        ),
        contract,
    };

    ImageDecodePlan {
        decision,
        capability_report,
        requires_source_region_decode,
        requires_reduction_decode,
        requires_component_decode,
        cache_key,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn test_viewport() -> Viewport {
        // A 100x100 pixel viewport at 72 DPI for a [0 0 100 100] media box.
        Viewport::new([0.0, 0.0, 100.0, 100.0], 72)
    }

    fn small_tile_viewport() -> Viewport {
        // A viewport representing a small tile: 50x50 pixels at offset (0,0)
        // for a [0 0 100 100] media box at 72 DPI, then restricted.
        let mut vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        vp.width_px = 50;
        vp.height_px = 50;
        vp.origin_x_px = 0;
        vp.origin_y_px = 0;
        vp
    }

    fn metadata_for_test() -> ImageMetadata {
        ImageMetadata {
            object_number: 5,
            generation_number: 0,
            width: 200,
            height: 200,
            bits_per_component: 8,
            color_space: "DeviceRGB".to_string(),
            filters: vec!["FlateDecode".to_string()],
            is_mask: false,
            is_inline: false,
            requires_full_image_postprocessing: false,
            decode_fingerprint: "none".to_string(),
            decode_params_fingerprint: "none".to_string(),
            image_mask_fingerprint: "none".to_string(),
            soft_mask_fingerprint: "none".to_string(),
            interpolate: false,
            component_selection: ImageComponentSelection::All,
        }
    }

    #[test]
    fn image_inside_viewport_requires_decode() {
        let vp = test_viewport();
        // CTM places image at [10,10] with size 50x50 in page space
        let ctm = Transform2D::new(50.0, 0.0, 0.0, 50.0, 10.0, 10.0);
        let meta = metadata_for_test();
        let plan = plan_image_decode(&meta, &ctm, &vp, "test:5:0:200:200:8:FlateDecode", false);
        assert_eq!(plan.decision, ImageDecodePlanDecision::DecodeRequired);
    }

    #[test]
    fn image_outside_viewport_skips_decode() {
        let vp = small_tile_viewport();
        // CTM places image entirely at x=[70..90], y=[70..90] in page space.
        // With a 50x50 pixel viewport at origin (0,0), the device pixels
        // for this image will be beyond pixel 50, hence outside.
        let ctm = Transform2D::new(20.0, 0.0, 0.0, 20.0, 70.0, 10.0);
        let meta = metadata_for_test();
        let plan = plan_image_decode(&meta, &ctm, &vp, "test:5:0:200:200:8:FlateDecode", false);
        assert_eq!(plan.decision, ImageDecodePlanDecision::SkipOutsideViewport);
    }

    #[test]
    fn cache_key_differs_for_different_target_dimensions() {
        let vp = test_viewport();
        let meta = metadata_for_test();
        let base = "xobject:5:0:200:200:8:FlateDecode";

        // Small CTM
        let ctm_small = Transform2D::new(30.0, 0.0, 0.0, 30.0, 10.0, 10.0);
        let plan_small = plan_image_decode(&meta, &ctm_small, &vp, base, false);

        // Large CTM
        let ctm_large = Transform2D::new(80.0, 0.0, 0.0, 80.0, 10.0, 10.0);
        let plan_large = plan_image_decode(&meta, &ctm_large, &vp, base, false);

        let key_small = plan_small.cache_key.to_cache_string();
        let key_large = plan_large.cache_key.to_cache_string();
        assert_ne!(
            key_small, key_large,
            "cache keys must differ for different target sizes"
        );
    }

    #[test]
    fn cache_key_differs_for_quality_mode() {
        let vp = test_viewport();
        let meta = metadata_for_test();
        let base = "xobject:5:0:200:200:8:FlateDecode";
        let ctm = Transform2D::new(50.0, 0.0, 0.0, 50.0, 10.0, 10.0);

        let plan_compat = plan_image_decode(&meta, &ctm, &vp, base, false);
        let plan_hq = plan_image_decode(&meta, &ctm, &vp, base, true);

        assert_ne!(
            plan_compat.cache_key.to_cache_string(),
            plan_hq.cache_key.to_cache_string(),
            "cache keys must differ for different quality modes"
        );
    }

    #[test]
    fn cache_key_includes_render_contract_identity() {
        let vp = test_viewport();
        let meta = metadata_for_test();
        let base = "xobject:5:0:200:200:8:FlateDecode";
        let ctm = Transform2D::new(50.0, 0.0, 0.0, 50.0, 10.0, 10.0);
        let backend = BackendSelection::StandardCpu;

        let contract_a = plan_image_decode_with_identity(
            &meta,
            &ctm,
            &vp,
            base,
            false,
            ImageDecodePlanIdentity::for_backend(backend, "contract-a"),
        )
        .cache_key
        .to_cache_string();
        let contract_b = plan_image_decode_with_identity(
            &meta,
            &ctm,
            &vp,
            base,
            false,
            ImageDecodePlanIdentity::for_backend(backend, "contract-b"),
        )
        .cache_key
        .to_cache_string();

        assert!(contract_a.contains(":contract:contract-a:"), "{contract_a}");
        assert_ne!(
            contract_a, contract_b,
            "image decode cache keys must not collide across render contracts"
        );
    }

    #[test]
    fn cache_key_differs_for_jpx_filter() {
        let vp = test_viewport();
        let base = "xobject:5:0:200:200:8:JPXDecode";
        let ctm = Transform2D::new(50.0, 0.0, 0.0, 50.0, 10.0, 10.0);

        let meta_flate = metadata_for_test();
        let mut meta_jpx = metadata_for_test();
        meta_jpx.filters = vec!["JPXDecode".to_string()];

        let plan_flate = plan_image_decode(&meta_flate, &ctm, &vp, base, false);
        let plan_jpx = plan_image_decode(&meta_jpx, &ctm, &vp, base, false);

        assert_ne!(
            plan_flate.cache_key.to_cache_string(),
            plan_jpx.cache_key.to_cache_string(),
            "cache keys must differ for JPX vs non-JPX"
        );
    }

    #[test]
    fn jpx_plan_reports_full_decode_only_capability() {
        let vp = test_viewport();
        let base = "xobject:5:0:200:200:8:JPXDecode";
        let ctm = Transform2D::new(200.0, 0.0, 0.0, 200.0, 10.0, 10.0);
        let mut meta = metadata_for_test();
        meta.filters = vec!["JPXDecode".to_string()];

        let plan = plan_image_decode(&meta, &ctm, &vp, base, false);

        assert_eq!(plan.capability_report.codec, ImageDecodeCodec::Jpx);
        assert_eq!(
            plan.capability_report.metadata_inspection,
            ImageDecodeCapabilityStatus::Native
        );
        assert_eq!(
            plan.capability_report.region_decode,
            ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::DecoderApiUnavailable
            )
        );
        assert_eq!(
            plan.capability_report.reduction_decode,
            ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::DecoderApiUnavailable
            )
        );
        assert_eq!(
            plan.capability_report.progressive_decode,
            ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::DecoderApiUnavailable
            )
        );
        assert_eq!(
            plan.capability_report.tile_decode,
            ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::CodestreamTileDecodeUnavailable
            )
        );
        assert_eq!(
            plan.capability_report.component_decode,
            ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::ComponentSelectionUnavailable
            )
        );
        assert_eq!(
            plan.capability_report.cancellation,
            ImageDecodeExecutionControlStatus::RendererBoundary
        );
        assert_eq!(
            plan.capability_report.memory_budget,
            ImageDecodeExecutionControlStatus::RendererBoundary
        );
        assert!(plan.capability_report.requires_full_decode());
        assert!(!plan.requires_reduction_decode);
        assert_eq!(
            plan.cache_key.contract.source_region,
            plan.capability_report.source_region
        );
        assert_eq!(
            plan.cache_key.contract.reduction_level,
            plan.capability_report.reduction_level
        );
    }

    #[test]
    fn unknown_filter_in_chain_prevents_terminal_codec_capability_claim() {
        let vp = small_tile_viewport();
        let base = "xobject:5:0:50:50:1:VendorDecode+CCITTFaxDecode";
        let ctm = Transform2D::new(75.0, 0.0, 0.0, 75.0, -20.0, -20.0);
        let mut meta = metadata_for_test();
        meta.width = 50;
        meta.height = 50;
        meta.bits_per_component = 1;
        meta.color_space = "DeviceGray".to_string();
        meta.filters = vec!["VendorDecode".to_string(), "CCITTFaxDecode".to_string()];

        let plan = plan_image_decode(&meta, &ctm, &vp, base, false);

        assert_eq!(plan.capability_report.codec, ImageDecodeCodec::Unknown);
        assert_eq!(
            plan.capability_report.region_decode,
            ImageDecodeCapabilityStatus::Unavailable(ImageDecodeUnavailableReason::UnknownCodec)
        );
        assert_eq!(
            plan.capability_report.source_region,
            ImageSourceRegion::Full
        );
        assert!(
            plan.requires_unavailable_exact_decode_support(),
            "unknown wrappers must not inherit native CCITT source-window capability"
        );
    }

    #[test]
    fn jpeg_downscale_plan_reports_native_reduction_capability() {
        let vp = test_viewport();
        let base = "xobject:5:0:200:200:8:DCTDecode";
        let ctm = Transform2D::new(50.0, 0.0, 0.0, 50.0, 10.0, 10.0);
        let mut meta = metadata_for_test();
        meta.filters = vec!["DCTDecode".to_string()];

        let plan = plan_image_decode(&meta, &ctm, &vp, base, false);

        assert_eq!(plan.capability_report.codec, ImageDecodeCodec::Jpeg);
        assert!(plan.requires_reduction_decode);
        assert!(!plan.requires_component_decode);
        assert_eq!(
            plan.capability_report.reduction_decode,
            ImageDecodeCapabilityStatus::Native
        );
        assert_eq!(
            plan.capability_report.reduction_level,
            ImageReductionLevel::PowerOfTwo(2)
        );
        assert_eq!(
            plan.cache_key.contract.reduction_level,
            ImageReductionLevel::PowerOfTwo(2)
        );
        assert!(!plan.requires_unavailable_exact_decode_support());
        assert!(plan
            .cache_key
            .to_cache_string()
            .contains(":codec:jpeg:sr:full:rl:pow2:2"));
    }

    #[test]
    fn jpx_downscale_plan_reports_native_reduction_capability() {
        let vp = test_viewport();
        let base = "xobject:5:0:200:200:8:JPXDecode";
        let ctm = Transform2D::new(50.0, 0.0, 0.0, 50.0, 10.0, 10.0);
        let mut meta = metadata_for_test();
        meta.filters = vec!["JPXDecode".to_string()];

        let plan = plan_image_decode(&meta, &ctm, &vp, base, false);

        assert_eq!(plan.capability_report.codec, ImageDecodeCodec::Jpx);
        assert!(plan.requires_reduction_decode);
        assert!(!plan.requires_component_decode);
        assert_eq!(
            plan.capability_report.reduction_decode,
            ImageDecodeCapabilityStatus::Native
        );
        assert_eq!(
            plan.capability_report.region_decode,
            ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::DecoderApiUnavailable
            )
        );
        assert_eq!(
            plan.capability_report.progressive_decode,
            ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::DecoderApiUnavailable
            )
        );
        assert_eq!(
            plan.capability_report.tile_decode,
            ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::CodestreamTileDecodeUnavailable
            )
        );
        assert_eq!(
            plan.capability_report.reduction_level,
            ImageReductionLevel::PowerOfTwo(2)
        );
        assert_eq!(
            plan.cache_key.contract.reduction_level,
            ImageReductionLevel::PowerOfTwo(2)
        );
        assert!(!plan.requires_unavailable_exact_decode_support());
        assert!(!plan.capability_report.requires_full_decode());
        assert!(plan
            .cache_key
            .to_cache_string()
            .contains(":codec:jpx:sr:full:rl:pow2:2"));
    }

    #[test]
    fn jpx_abbreviation_downscale_plan_reports_native_reduction_capability() {
        let vp = test_viewport();
        let base = "xobject:5:0:200:200:8:JPX";
        let ctm = Transform2D::new(50.0, 0.0, 0.0, 50.0, 10.0, 10.0);
        let mut meta = metadata_for_test();
        meta.filters = vec!["JPX".to_string()];

        let plan = plan_image_decode(&meta, &ctm, &vp, base, false);

        assert_eq!(plan.capability_report.codec, ImageDecodeCodec::Jpx);
        assert!(plan.requires_reduction_decode);
        assert!(!plan.requires_component_decode);
        assert_eq!(
            plan.capability_report.reduction_decode,
            ImageDecodeCapabilityStatus::Native
        );
        assert_eq!(
            plan.capability_report.reduction_level,
            ImageReductionLevel::PowerOfTwo(2)
        );
        assert!(!plan.requires_unavailable_exact_decode_support());
        assert!(plan
            .cache_key
            .to_cache_string()
            .contains(":codec:jpx:sr:full:rl:pow2:2"));
    }

    #[test]
    fn native_reduction_requires_both_axes_to_cover_target() {
        let vp = Viewport::new([0.0, 0.0, 1000.0, 1000.0], 72);
        let ctm = Transform2D::new(900.0, 0.0, 0.0, 4.0, 10.0, 10.0);
        let mut meta = metadata_for_test();
        meta.width = 1000;
        meta.height = 16;

        for (filter, codec) in [
            ("DCTDecode", ImageDecodeCodec::Jpeg),
            ("JPXDecode", ImageDecodeCodec::Jpx),
        ] {
            meta.filters = vec![filter.to_string()];
            let plan = plan_image_decode(
                &meta,
                &ctm,
                &vp,
                &format!("xobject:5:0:1000:16:8:{filter}"),
                false,
            );

            assert_eq!(plan.capability_report.codec, codec);
            assert!(plan.requires_reduction_decode);
            assert_eq!(
                plan.capability_report.reduction_level,
                ImageReductionLevel::None,
                "{filter} must not pick a reduced tier that undersamples the wide axis"
            );
            assert_eq!(
                plan.capability_report.reduction_decode,
                ImageDecodeCapabilityStatus::Unavailable(
                    ImageDecodeUnavailableReason::DecoderApiUnavailable
                )
            );
            assert!(plan.requires_unavailable_exact_decode_support());
            assert_eq!(plan.exact_decode_limitation_summary(), "reduced-resolution");
            assert!(
                plan.cache_key.to_cache_string().contains(":rl:none"),
                "unsafe reduced tiers must not enter cache identity"
            );
        }
    }

    #[test]
    fn jpeg_downscale_plan_keeps_reduction_unavailable_when_postprocessing_needs_full_image() {
        let vp = test_viewport();
        let base = "xobject:5:0:200:200:8:DCTDecode";
        let ctm = Transform2D::new(50.0, 0.0, 0.0, 50.0, 10.0, 10.0);
        let mut meta = metadata_for_test();
        meta.filters = vec!["DCTDecode".to_string()];
        meta.requires_full_image_postprocessing = true;

        let plan = plan_image_decode(&meta, &ctm, &vp, base, true);

        assert_eq!(plan.capability_report.codec, ImageDecodeCodec::Jpeg);
        assert!(plan.requires_reduction_decode);
        assert_eq!(
            plan.capability_report.reduction_decode,
            ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::DecoderApiUnavailable
            )
        );
        assert_eq!(
            plan.capability_report.reduction_level,
            ImageReductionLevel::None
        );
        assert!(plan.requires_unavailable_exact_decode_support());
        assert_eq!(plan.exact_decode_limitation_summary(), "reduced-resolution");
    }

    #[test]
    fn jpx_downscale_plan_keeps_reduction_unavailable_when_postprocessing_needs_full_image() {
        let vp = test_viewport();
        let base = "xobject:5:0:200:200:8:JPXDecode";
        let ctm = Transform2D::new(50.0, 0.0, 0.0, 50.0, 10.0, 10.0);
        let mut meta = metadata_for_test();
        meta.filters = vec!["JPXDecode".to_string()];
        meta.requires_full_image_postprocessing = true;

        let plan = plan_image_decode(&meta, &ctm, &vp, base, true);

        assert_eq!(plan.capability_report.codec, ImageDecodeCodec::Jpx);
        assert!(plan.requires_reduction_decode);
        assert_eq!(
            plan.capability_report.reduction_decode,
            ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::DecoderApiUnavailable
            )
        );
        assert_eq!(
            plan.capability_report.reduction_level,
            ImageReductionLevel::None
        );
        assert!(plan.requires_unavailable_exact_decode_support());
        assert_eq!(plan.exact_decode_limitation_summary(), "reduced-resolution");
    }

    #[test]
    fn ccitt_plan_uses_windowed_source_region_when_clipped() {
        let vp = test_viewport();
        let base = "xobject:5:0:50:50:1:CCITTFaxDecode";
        let ctm = Transform2D::new(150.0, 0.0, 0.0, 150.0, -25.0, -25.0);
        let mut meta = metadata_for_test();
        meta.width = 50;
        meta.height = 50;
        meta.bits_per_component = 1;
        meta.color_space = "DeviceGray".to_string();
        meta.filters = vec!["CCITTFaxDecode".to_string()];

        let plan = plan_image_decode(&meta, &ctm, &vp, base, false);

        assert_eq!(plan.capability_report.codec, ImageDecodeCodec::Ccitt);
        assert!(plan.requires_source_region_decode);
        assert!(!plan.requires_reduction_decode);
        assert_eq!(
            plan.capability_report.region_decode,
            ImageDecodeCapabilityStatus::Native
        );
        assert!(matches!(
            plan.capability_report.source_region,
            ImageSourceRegion::SubRect { .. }
        ));
        assert!(!plan.capability_report.requires_full_decode());
        assert!(!plan.requires_unavailable_exact_decode_support());
        assert_eq!(
            plan.cache_key.contract.source_region,
            plan.capability_report.source_region
        );
    }

    #[test]
    fn ccitt_plan_does_not_advertise_window_decode_for_non_monochrome_shape() {
        let vp = test_viewport();
        let base = "xobject:5:0:50:50:1:CCITTFaxDecode";
        let ctm = Transform2D::new(150.0, 0.0, 0.0, 150.0, -25.0, -25.0);
        let mut meta = metadata_for_test();
        meta.width = 50;
        meta.height = 50;
        meta.bits_per_component = 1;
        meta.color_space = "DeviceRGB".to_string();
        meta.filters = vec!["CCITTFaxDecode".to_string()];

        let plan = plan_image_decode(&meta, &ctm, &vp, base, false);

        assert_eq!(plan.capability_report.codec, ImageDecodeCodec::Ccitt);
        assert!(plan.requires_source_region_decode);
        assert_eq!(
            plan.capability_report.source_region,
            ImageSourceRegion::Full
        );
        assert_eq!(
            plan.capability_report.region_decode,
            ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::MonochromeTerminalShapeUnsupported
            )
        );
        assert!(plan.requires_unavailable_exact_decode_support());
        assert_eq!(plan.exact_decode_limitation_summary(), "source-region");
    }

    #[test]
    fn ccitt_image_reference_capability_reports_non_monochrome_shape() {
        let image = ImageReference {
            page_number: 1,
            xobject_name: "ImCcittRgb".to_string(),
            object_number: 5,
            generation_number: 0,
            width: 50,
            height: 50,
            bits_per_component: 1,
            color_space: "DeviceRGB".to_string(),
            filter: vec!["CCITTFaxDecode".to_string()],
            is_inline: false,
            is_mask: false,
            is_smask: false,
            inline_data: None,
        };

        let report = image_decode_capabilities_for_image_reference(&image);

        assert_eq!(report.codec, ImageDecodeCodec::Ccitt);
        assert_eq!(
            report.region_decode,
            ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::MonochromeTerminalShapeUnsupported
            )
        );
        assert!(report.requires_full_decode());
    }

    #[test]
    fn ccitt_plan_keeps_region_unavailable_when_postprocessing_needs_full_image() {
        let vp = test_viewport();
        let base = "xobject:5:0:50:50:1:CCITTFaxDecode";
        let ctm = Transform2D::new(150.0, 0.0, 0.0, 150.0, -25.0, -25.0);
        let mut meta = metadata_for_test();
        meta.width = 50;
        meta.height = 50;
        meta.bits_per_component = 1;
        meta.color_space = "DeviceGray".to_string();
        meta.filters = vec!["CCITTFaxDecode".to_string()];
        meta.requires_full_image_postprocessing = true;

        let plan = plan_image_decode(&meta, &ctm, &vp, base, false);

        assert_eq!(plan.capability_report.codec, ImageDecodeCodec::Ccitt);
        assert!(plan.requires_source_region_decode);
        assert_eq!(
            plan.capability_report.source_region,
            ImageSourceRegion::Full
        );
        assert_eq!(
            plan.capability_report.region_decode,
            ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::InternalWindowedDecodeNotIntegrated
            )
        );
        assert!(plan.requires_unavailable_exact_decode_support());
    }

    #[test]
    fn ccitt_smask_plan_uses_windowed_source_region_when_crop_aligned() {
        let vp = test_viewport();
        let base = "xobject:5:0:50:50:1:CCITTFaxDecode-smask";
        let ctm = Transform2D::new(150.0, 0.0, 0.0, 150.0, -25.0, -25.0);
        let mut meta = metadata_for_test();
        meta.width = 50;
        meta.height = 50;
        meta.bits_per_component = 1;
        meta.color_space = "DeviceGray".to_string();
        meta.filters = vec!["CCITTFaxDecode".to_string()];
        meta.soft_mask_fingerprint = "ref:6:0".to_string();
        meta.requires_full_image_postprocessing = false;

        let plan = plan_image_decode(&meta, &ctm, &vp, base, false);

        assert_eq!(plan.capability_report.codec, ImageDecodeCodec::Ccitt);
        assert!(plan.requires_source_region_decode);
        assert_eq!(
            plan.capability_report.region_decode,
            ImageDecodeCapabilityStatus::Native
        );
        assert!(matches!(
            plan.capability_report.source_region,
            ImageSourceRegion::SubRect { .. }
        ));
        assert_eq!(plan.cache_key.contract.soft_mask_fingerprint, "ref:6:0");
        assert_eq!(
            plan.cache_key.contract.source_region,
            plan.capability_report.source_region
        );
        assert!(!plan.requires_unavailable_exact_decode_support());
    }

    #[test]
    fn ccitt_plan_uses_inline_windowed_source_region_when_clipped() {
        let vp = test_viewport();
        let base = "inline:page:1:50x50:1:DeviceGray:CCITTFaxDecode";
        let ctm = Transform2D::new(150.0, 0.0, 0.0, 150.0, -25.0, -25.0);
        let mut meta = metadata_for_test();
        meta.object_number = 0;
        meta.width = 50;
        meta.height = 50;
        meta.bits_per_component = 1;
        meta.color_space = "DeviceGray".to_string();
        meta.filters = vec!["CCITTFaxDecode".to_string()];
        meta.is_inline = true;

        let plan = plan_image_decode(&meta, &ctm, &vp, base, false);

        assert_eq!(plan.capability_report.codec, ImageDecodeCodec::Ccitt);
        assert!(plan.requires_source_region_decode);
        assert_eq!(
            plan.capability_report.region_decode,
            ImageDecodeCapabilityStatus::Native
        );
        assert!(matches!(
            plan.capability_report.source_region,
            ImageSourceRegion::SubRect { .. }
        ));
        assert!(!plan.capability_report.requires_full_decode());
        assert!(!plan.requires_unavailable_exact_decode_support());
        assert_eq!(
            plan.cache_key.contract.source_region,
            plan.capability_report.source_region
        );
    }

    #[test]
    fn raw_unfiltered_xobject_plan_uses_windowed_source_region_when_clipped() {
        let vp = test_viewport();
        let base = "xobject:5:0:50:50:8:raw";
        let ctm = Transform2D::new(150.0, 0.0, 0.0, 150.0, -25.0, -25.0);
        let mut meta = metadata_for_test();
        meta.width = 50;
        meta.height = 50;
        meta.bits_per_component = 8;
        meta.filters = vec![];

        let plan = plan_image_decode(&meta, &ctm, &vp, base, false);

        assert_eq!(plan.capability_report.codec, ImageDecodeCodec::Raw);
        assert!(plan.requires_source_region_decode);
        assert!(!plan.requires_reduction_decode);
        assert_eq!(
            plan.capability_report.region_decode,
            ImageDecodeCapabilityStatus::Native
        );
        assert!(matches!(
            plan.capability_report.source_region,
            ImageSourceRegion::SubRect { .. }
        ));
        assert!(!plan.capability_report.requires_full_decode());
        assert!(!plan.requires_unavailable_exact_decode_support());
        assert_eq!(
            plan.cache_key.contract.source_region,
            plan.capability_report.source_region
        );
        assert!(plan.cache_key.to_cache_string().contains(":codec:raw:"));
    }

    #[test]
    fn raw_unfiltered_subbyte_xobject_plan_uses_windowed_source_region_when_clipped() {
        let vp = test_viewport();
        let base = "xobject:5:0:50:50:1:raw";
        let ctm = Transform2D::new(150.0, 0.0, 0.0, 150.0, -25.0, -25.0);
        let mut meta = metadata_for_test();
        meta.width = 50;
        meta.height = 50;
        meta.bits_per_component = 1;
        meta.filters = vec![];

        let plan = plan_image_decode(&meta, &ctm, &vp, base, false);

        assert_eq!(plan.capability_report.codec, ImageDecodeCodec::Raw);
        assert!(plan.requires_source_region_decode);
        assert!(!plan.requires_reduction_decode);
        assert_eq!(
            plan.capability_report.region_decode,
            ImageDecodeCapabilityStatus::Native
        );
        assert_eq!(
            plan.cache_key.contract.source_region,
            plan.capability_report.source_region
        );
        assert!(!plan.requires_unavailable_exact_decode_support());
    }

    #[test]
    fn raw_window_plan_refuses_filtered_or_unsafe_shapes() {
        let vp = test_viewport();
        let base = "xobject:5:0:50:50:8:raw";
        let ctm = Transform2D::new(150.0, 0.0, 0.0, 150.0, -25.0, -25.0);
        let mut filtered = metadata_for_test();
        filtered.width = 50;
        filtered.height = 50;
        filtered.filters = vec!["FlateDecode".to_string()];

        let filtered_plan = plan_image_decode(&filtered, &ctm, &vp, base, false);
        assert_eq!(
            filtered_plan.capability_report.codec,
            ImageDecodeCodec::Lossless
        );
        assert_eq!(
            filtered_plan.capability_report.region_decode,
            ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::GenericStreamFilterFullMaterialization
            )
        );
        assert!(filtered_plan.requires_unavailable_exact_decode_support());

        let mut raw_mask = metadata_for_test();
        raw_mask.width = 50;
        raw_mask.height = 50;
        raw_mask.bits_per_component = 4;
        raw_mask.filters = vec![];
        raw_mask.is_mask = true;
        let mask_plan = plan_image_decode(&raw_mask, &ctm, &vp, base, false);
        assert_eq!(mask_plan.capability_report.codec, ImageDecodeCodec::Raw);
        assert_eq!(
            mask_plan.capability_report.region_decode,
            ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::RawWindowUnsupportedShape
            )
        );
        assert!(mask_plan.requires_unavailable_exact_decode_support());
    }

    #[test]
    fn raw_window_unfiltered_image_mask_plan_uses_windowed_source_region_when_clipped() {
        let vp = test_viewport();
        let base = "xobject:5:0:50:50:1:raw-mask";
        let ctm = Transform2D::new(150.0, 0.0, 0.0, 150.0, -25.0, -25.0);
        let mut meta = metadata_for_test();
        meta.width = 50;
        meta.height = 50;
        meta.bits_per_component = 1;
        meta.filters = vec![];
        meta.is_mask = true;
        meta.color_space = "DeviceGray".to_string();
        meta.image_mask_fingerprint = "stencil".to_string();

        let plan = plan_image_decode(&meta, &ctm, &vp, base, false);

        assert_eq!(plan.capability_report.codec, ImageDecodeCodec::Raw);
        assert!(plan.requires_source_region_decode);
        assert_eq!(
            plan.capability_report.region_decode,
            ImageDecodeCapabilityStatus::Native
        );
        assert!(matches!(
            plan.capability_report.source_region,
            ImageSourceRegion::SubRect { .. }
        ));
        assert_eq!(plan.cache_key.contract.image_mask_fingerprint, "stencil");
        assert!(!plan.requires_unavailable_exact_decode_support());
    }

    #[test]
    fn raw_unfiltered_explicit_mask_plan_uses_windowed_source_region_when_crop_aligned() {
        let vp = test_viewport();
        let base = "xobject:5:0:50:50:8:raw-explicit-mask";
        let ctm = Transform2D::new(150.0, 0.0, 0.0, 150.0, -25.0, -25.0);
        let mut meta = metadata_for_test();
        meta.width = 50;
        meta.height = 50;
        meta.bits_per_component = 8;
        meta.filters = vec![];
        meta.image_mask_fingerprint = "ref:6:0".to_string();
        meta.requires_full_image_postprocessing = false;

        let plan = plan_image_decode(&meta, &ctm, &vp, base, false);

        assert_eq!(plan.capability_report.codec, ImageDecodeCodec::Raw);
        assert!(plan.requires_source_region_decode);
        assert_eq!(
            plan.capability_report.region_decode,
            ImageDecodeCapabilityStatus::Native
        );
        assert!(matches!(
            plan.capability_report.source_region,
            ImageSourceRegion::SubRect { .. }
        ));
        assert_eq!(plan.cache_key.contract.image_mask_fingerprint, "ref:6:0");
        assert_eq!(
            plan.cache_key.contract.source_region,
            plan.capability_report.source_region
        );
        assert!(!plan.requires_unavailable_exact_decode_support());
    }

    #[test]
    fn raw_unfiltered_smask_plan_uses_windowed_source_region_when_crop_aligned() {
        let vp = test_viewport();
        let base = "xobject:5:0:50:50:8:raw-smask";
        let ctm = Transform2D::new(150.0, 0.0, 0.0, 150.0, -25.0, -25.0);
        let mut meta = metadata_for_test();
        meta.width = 50;
        meta.height = 50;
        meta.bits_per_component = 8;
        meta.filters = vec![];
        meta.soft_mask_fingerprint = "ref:6:0".to_string();
        meta.requires_full_image_postprocessing = false;

        let plan = plan_image_decode(&meta, &ctm, &vp, base, false);

        assert_eq!(plan.capability_report.codec, ImageDecodeCodec::Raw);
        assert!(plan.requires_source_region_decode);
        assert_eq!(
            plan.capability_report.region_decode,
            ImageDecodeCapabilityStatus::Native
        );
        assert!(matches!(
            plan.capability_report.source_region,
            ImageSourceRegion::SubRect { .. }
        ));
        assert_eq!(plan.cache_key.contract.soft_mask_fingerprint, "ref:6:0");
        assert_eq!(
            plan.cache_key.contract.source_region,
            plan.capability_report.source_region
        );
        assert!(!plan.requires_unavailable_exact_decode_support());
    }

    #[test]
    fn raw_unfiltered_inline_plan_uses_windowed_source_region_when_clipped() {
        let vp = test_viewport();
        let base = "inline:rev:1:raw";
        let ctm = Transform2D::new(150.0, 0.0, 0.0, 150.0, -25.0, -25.0);
        let mut meta = metadata_for_test();
        meta.width = 50;
        meta.height = 50;
        meta.filters = vec![];
        meta.is_inline = true;

        let plan = plan_image_decode(&meta, &ctm, &vp, base, false);

        assert_eq!(plan.capability_report.codec, ImageDecodeCodec::Raw);
        assert_eq!(
            plan.capability_report.region_decode,
            ImageDecodeCapabilityStatus::Native
        );
        assert_eq!(
            plan.cache_key.contract.source_region,
            ImageSourceRegion::SubRect {
                x: 7,
                y: 7,
                width: 36,
                height: 36,
            }
        );
        assert!(!plan.requires_unavailable_exact_decode_support());
    }

    #[test]
    fn visible_clipped_image_marks_source_region_requirement() {
        let vp = test_viewport();
        let base = "xobject:5:0:200:200:8:FlateDecode";
        let ctm = Transform2D::new(150.0, 0.0, 0.0, 150.0, -25.0, -25.0);
        let mut meta = metadata_for_test();
        meta.width = 100;
        meta.height = 100;

        let plan = plan_image_decode(&meta, &ctm, &vp, base, true);

        assert_eq!(plan.decision, ImageDecodePlanDecision::DecodeRequired);
        assert!(plan.requires_source_region_decode);
        assert!(plan.requires_unavailable_exact_decode_support());
        assert_eq!(plan.exact_decode_limitation_summary(), "source-region");
    }

    #[test]
    fn downscaled_image_marks_reduction_requirement() {
        let vp = test_viewport();
        let base = "xobject:5:0:200:200:8:FlateDecode";
        let ctm = Transform2D::new(50.0, 0.0, 0.0, 50.0, 10.0, 10.0);
        let meta = metadata_for_test();

        let plan = plan_image_decode(&meta, &ctm, &vp, base, true);

        assert_eq!(plan.decision, ImageDecodePlanDecision::DecodeRequired);
        assert!(!plan.requires_source_region_decode);
        assert!(plan.requires_reduction_decode);
        assert!(plan.requires_unavailable_exact_decode_support());
        assert_eq!(plan.exact_decode_limitation_summary(), "reduced-resolution");
    }

    #[test]
    fn progressive_image_decode_session_reports_full_decode_required() {
        let vp = test_viewport();
        let base = "xobject:5:0:200:200:8:JPXDecode";
        let ctm = Transform2D::new(200.0, 0.0, 0.0, 200.0, 10.0, 10.0);
        let mut meta = metadata_for_test();
        meta.filters = vec!["JPXDecode".to_string()];
        let plan = plan_image_decode(&meta, &ctm, &vp, base, false);
        let request = ProgressiveImageDecodeRequest::from_plan(&plan, 4096);
        let mut session = ProgressiveImageDecodeSession::new(request);

        let start = session.start();
        assert_eq!(start.state, ProgressiveImageDecodeState::Started);
        assert!(start.resumable);
        assert!(!start.native_progressive_available);

        let report = session.continue_decode();
        assert_eq!(report.phase, "full_decode_required");
        assert_eq!(report.state, ProgressiveImageDecodeState::Started);
        assert!(report.full_decode_required);
        assert_eq!(
            report.unavailable_reason,
            Some(ImageDecodeUnavailableReason::DecoderApiUnavailable)
        );
        assert!(!report.completed);
    }

    #[test]
    fn progressive_full_raw_plan_reports_full_decode_required_even_with_window_capability() {
        let vp = test_viewport();
        let base = "xobject:5:0:50:50:8:raw";
        let ctm = Transform2D::new(50.0, 0.0, 0.0, 50.0, 10.0, 10.0);
        let mut meta = metadata_for_test();
        meta.width = 50;
        meta.height = 50;
        meta.filters.clear();

        let plan = plan_image_decode(&meta, &ctm, &vp, base, false);
        assert_eq!(plan.capability_report.codec, ImageDecodeCodec::Raw);
        assert_eq!(
            plan.capability_report.region_decode,
            ImageDecodeCapabilityStatus::Native
        );
        assert_eq!(
            plan.capability_report.source_region,
            ImageSourceRegion::Full
        );
        assert!(!plan.requires_source_region_decode);
        assert!(!plan.requires_reduction_decode);
        assert!(plan.capability_report.requires_full_decode());

        let request = ProgressiveImageDecodeRequest::from_plan(&plan, 4096);
        let mut session = ProgressiveImageDecodeSession::new(request);
        let report = session.continue_decode();

        assert_eq!(report.phase, "full_decode_required");
        assert_eq!(report.state, ProgressiveImageDecodeState::Started);
        assert!(!report.native_progressive_available);
        assert!(report.full_decode_required);
        assert_eq!(
            report.unavailable_reason,
            Some(ImageDecodeUnavailableReason::DecoderApiUnavailable)
        );
    }

    #[test]
    fn progressive_native_window_plan_completes_without_full_decode_requirement() {
        let vp = test_viewport();
        let base = "xobject:5:0:50:50:8:raw-window-progressive";
        let ctm = Transform2D::new(150.0, 0.0, 0.0, 150.0, -25.0, -25.0);
        let mut meta = metadata_for_test();
        meta.width = 50;
        meta.height = 50;
        meta.bits_per_component = 8;
        meta.filters.clear();

        let plan = plan_image_decode(&meta, &ctm, &vp, base, false);
        assert!(plan.requires_source_region_decode);
        assert_eq!(
            plan.capability_report.source_region,
            ImageSourceRegion::SubRect {
                x: 7,
                y: 7,
                width: 36,
                height: 36,
            }
        );
        assert!(plan.capability_report.uses_native_bounded_decode());

        let request = ProgressiveImageDecodeRequest::from_plan(&plan, 4096);
        let mut session = ProgressiveImageDecodeSession::new(request);
        let report = session.continue_decode();

        assert_eq!(report.phase, "planned_partial_decode_complete");
        assert_eq!(report.state, ProgressiveImageDecodeState::Completed);
        assert!(report.completed);
        assert!(!report.resumable);
        assert!(!report.native_progressive_available);
        assert!(!report.full_decode_required);
        assert_eq!(
            report.unavailable_reason,
            Some(ImageDecodeUnavailableReason::DecoderApiUnavailable)
        );
        assert_eq!(report.retained_bytes, 0);

        let terminal = session.continue_decode();
        assert_eq!(terminal.phase, "continue_terminal");
        assert_eq!(terminal.state, ProgressiveImageDecodeState::Completed);
        assert!(terminal.completed);
        assert!(!terminal.full_decode_required);
    }

    #[test]
    fn progressive_native_reduction_plan_completes_without_full_decode_requirement() {
        let vp = test_viewport();
        let base = "xobject:5:0:200:200:8:DCTDecode";
        let ctm = Transform2D::new(50.0, 0.0, 0.0, 50.0, 10.0, 10.0);
        let mut meta = metadata_for_test();
        meta.filters = vec!["DCTDecode".to_string()];

        let plan = plan_image_decode(&meta, &ctm, &vp, base, false);
        assert!(plan.requires_reduction_decode);
        assert_eq!(
            plan.capability_report.reduction_level,
            ImageReductionLevel::PowerOfTwo(2)
        );
        assert!(plan.capability_report.uses_native_bounded_decode());

        let request = ProgressiveImageDecodeRequest::from_plan(&plan, 4096);
        let mut session = ProgressiveImageDecodeSession::new(request);
        let report = session.continue_decode();

        assert_eq!(report.phase, "planned_partial_decode_complete");
        assert_eq!(report.state, ProgressiveImageDecodeState::Completed);
        assert_eq!(report.reduction_level, ImageReductionLevel::PowerOfTwo(2));
        assert!(report.completed);
        assert!(!report.resumable);
        assert!(!report.native_progressive_available);
        assert!(!report.full_decode_required);
        assert_eq!(
            report.unavailable_reason,
            Some(ImageDecodeUnavailableReason::DecoderApiUnavailable)
        );
    }

    #[test]
    fn progressive_native_component_subset_plan_completes_without_full_decode_requirement() {
        let vp = Viewport::new([0.0, 0.0, 300.0, 300.0], 72);
        let base = "xobject:5:0:200:200:8:raw-component-progressive";
        let ctm = Transform2D::new(200.0, 0.0, 0.0, 200.0, 10.0, 10.0);
        let mut meta = metadata_for_test();
        meta.filters.clear();
        meta.component_selection = ImageComponentSelection::Components(vec![0, 2]);

        let plan = plan_image_decode(&meta, &ctm, &vp, base, false);
        assert!(!plan.requires_source_region_decode);
        assert!(!plan.requires_reduction_decode);
        assert!(plan.requires_component_decode);
        assert_eq!(
            plan.capability_report.component_decode,
            ImageDecodeCapabilityStatus::Native
        );
        assert!(plan.capability_report.requires_full_decode());

        let request = ProgressiveImageDecodeRequest::from_plan(&plan, 4096);
        assert!(request.requires_component_decode);
        let mut session = ProgressiveImageDecodeSession::new(request);
        let start = session.start();
        assert!(!start.full_decode_required);
        assert!(start.requires_component_decode);

        let report = session.continue_decode();

        assert_eq!(report.phase, "planned_partial_decode_complete");
        assert_eq!(report.state, ProgressiveImageDecodeState::Completed);
        assert_eq!(report.source_region, ImageSourceRegion::Full);
        assert_eq!(report.reduction_level, ImageReductionLevel::None);
        assert!(report.requires_component_decode);
        assert!(report.completed);
        assert!(!report.resumable);
        assert!(!report.native_progressive_available);
        assert!(!report.full_decode_required);
        assert_eq!(
            report.unavailable_reason,
            Some(ImageDecodeUnavailableReason::DecoderApiUnavailable)
        );
    }

    #[test]
    fn progressive_image_decode_pause_resume_cancel_close_release_state() {
        let vp = test_viewport();
        let meta = metadata_for_test();
        let plan = plan_image_decode(
            &meta,
            &Transform2D::new(20.0, 0.0, 0.0, 20.0, 10.0, 10.0),
            &vp,
            "xobject:5:0:200:200:8:FlateDecode",
            false,
        );
        let request = ProgressiveImageDecodeRequest::from_plan(&plan, 1024);
        let mut session = ProgressiveImageDecodeSession::new(request);

        session.start();
        session.retained_bytes = 512;
        let paused = session.pause();
        assert_eq!(paused.state, ProgressiveImageDecodeState::Paused);
        assert!(paused.resumable);

        let resumed = session.resume();
        assert_eq!(resumed.state, ProgressiveImageDecodeState::Started);
        assert!(resumed.resumable);

        let cancelled = session.cancel();
        assert_eq!(cancelled.state, ProgressiveImageDecodeState::Cancelled);
        assert_eq!(cancelled.retained_bytes, 0);
        assert_eq!(
            cancelled.release_reason,
            Some(ProgressiveImageDecodeReleaseReason::Cancellation)
        );
        assert!(!cancelled.resumable);

        let closed = session.close();
        assert_eq!(closed.state, ProgressiveImageDecodeState::Closed);
        assert_eq!(closed.retained_bytes, 0);
        assert_eq!(
            closed.release_reason,
            Some(ProgressiveImageDecodeReleaseReason::SessionClose)
        );
        assert!(!closed.resumable);

        let request = ProgressiveImageDecodeRequest::from_plan(&plan, 1024);
        let mut failed_session = ProgressiveImageDecodeSession::new(request);
        failed_session.start();
        failed_session.retained_bytes = 768;
        let failed = failed_session.fail();
        assert_eq!(failed.state, ProgressiveImageDecodeState::Failed);
        assert_eq!(failed.retained_bytes, 0);
        assert_eq!(
            failed.release_reason,
            Some(ProgressiveImageDecodeReleaseReason::RenderFailure)
        );
        assert!(!failed.resumable);
        assert!(!failed.full_decode_required);
    }

    #[test]
    fn progressive_image_decode_document_close_releases_state() {
        let native_progressive_report = ImageDecodeCapabilityReport {
            codec: ImageDecodeCodec::Jpx,
            source_region: ImageSourceRegion::Full,
            reduction_level: ImageReductionLevel::None,
            metadata_inspection: ImageDecodeCapabilityStatus::Native,
            region_decode: ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::DecoderApiUnavailable,
            ),
            reduction_decode: ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::DecoderApiUnavailable,
            ),
            progressive_decode: ImageDecodeCapabilityStatus::Native,
            tile_decode: ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::CodestreamTileDecodeUnavailable,
            ),
            component_decode: ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::ComponentSelectionUnavailable,
            ),
            cancellation: ImageDecodeExecutionControlStatus::RendererBoundary,
            memory_budget: ImageDecodeExecutionControlStatus::RendererBoundary,
        };
        let request = ProgressiveImageDecodeRequest::new(
            "native-progressive-document-close",
            native_progressive_report,
            4096,
        );
        let mut session = ProgressiveImageDecodeSession::new(request);
        session.start();
        session.retained_bytes = 2048;

        let closed = session.close_for_document_close();
        assert_eq!(closed.state, ProgressiveImageDecodeState::Closed);
        assert_eq!(closed.phase, "document_close");
        assert_eq!(closed.retained_bytes, 0);
        assert_eq!(
            closed.release_reason,
            Some(ProgressiveImageDecodeReleaseReason::DocumentClose)
        );
        assert!(!closed.resumable);
        assert!(!closed.full_decode_required);

        let after_close_continue = session.continue_decode();
        assert_eq!(
            after_close_continue.state,
            ProgressiveImageDecodeState::Closed
        );
        assert_eq!(after_close_continue.phase, "continue_terminal");
        assert_eq!(after_close_continue.retained_bytes, 0);
        assert_eq!(
            after_close_continue.release_reason,
            Some(ProgressiveImageDecodeReleaseReason::DocumentClose)
        );
        assert!(!after_close_continue.resumable);
        assert!(!after_close_continue.full_decode_required);
    }

    #[test]
    fn progressive_image_decode_continue_preserves_terminal_states() {
        let native_progressive_report = ImageDecodeCapabilityReport {
            codec: ImageDecodeCodec::Jpx,
            source_region: ImageSourceRegion::Full,
            reduction_level: ImageReductionLevel::None,
            metadata_inspection: ImageDecodeCapabilityStatus::Native,
            region_decode: ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::DecoderApiUnavailable,
            ),
            reduction_decode: ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::DecoderApiUnavailable,
            ),
            progressive_decode: ImageDecodeCapabilityStatus::Native,
            tile_decode: ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::CodestreamTileDecodeUnavailable,
            ),
            component_decode: ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::ComponentSelectionUnavailable,
            ),
            cancellation: ImageDecodeExecutionControlStatus::RendererBoundary,
            memory_budget: ImageDecodeExecutionControlStatus::RendererBoundary,
        };
        let request = ProgressiveImageDecodeRequest::new(
            "native-progressive-terminal",
            native_progressive_report,
            1024,
        );

        let mut cancelled_session = ProgressiveImageDecodeSession::new(request.clone());
        cancelled_session.start();
        cancelled_session.cancel();
        let after_cancel_continue = cancelled_session.continue_decode();
        assert_eq!(
            after_cancel_continue.state,
            ProgressiveImageDecodeState::Cancelled
        );
        assert_eq!(after_cancel_continue.phase, "continue_terminal");
        assert!(!after_cancel_continue.completed);
        assert!(!after_cancel_continue.resumable);
        assert!(!after_cancel_continue.full_decode_required);
        assert_eq!(after_cancel_continue.retained_bytes, 0);

        let mut failed_session = ProgressiveImageDecodeSession::new(request.clone());
        failed_session.start();
        failed_session.fail();
        let after_fail_continue = failed_session.continue_decode();
        assert_eq!(
            after_fail_continue.state,
            ProgressiveImageDecodeState::Failed
        );
        assert_eq!(after_fail_continue.phase, "continue_terminal");
        assert!(!after_fail_continue.completed);
        assert!(!after_fail_continue.resumable);
        assert!(!after_fail_continue.full_decode_required);
        assert_eq!(after_fail_continue.retained_bytes, 0);

        let mut closed_session = ProgressiveImageDecodeSession::new(request);
        closed_session.start();
        closed_session.close();
        let after_close_continue = closed_session.continue_decode();
        assert_eq!(
            after_close_continue.state,
            ProgressiveImageDecodeState::Closed
        );
        assert_eq!(after_close_continue.phase, "continue_terminal");
        assert!(!after_close_continue.completed);
        assert!(!after_close_continue.resumable);
        assert!(!after_close_continue.full_decode_required);
        assert_eq!(after_close_continue.retained_bytes, 0);
    }

    #[test]
    fn progressive_image_decode_request_id_is_stable_for_plan_key() {
        let vp = test_viewport();
        let meta = metadata_for_test();
        let base = "xobject:5:0:200:200:8:FlateDecode";
        let ctm = Transform2D::new(50.0, 0.0, 0.0, 50.0, 10.0, 10.0);
        let plan = plan_image_decode(&meta, &ctm, &vp, base, false);

        let first = ProgressiveImageDecodeRequest::from_plan(&plan, 1024);
        let second = ProgressiveImageDecodeRequest::from_plan(&plan, 2048);

        assert_eq!(first.request_id, second.request_id);
        assert_eq!(first.cache_key, second.cache_key);
        assert_ne!(first.max_retained_bytes, second.max_retained_bytes);
    }

    #[test]
    fn cache_key_includes_source_region_and_reduction() {
        let vp = test_viewport();
        let meta = metadata_for_test();
        let base = "xobject:5:0:200:200:8:FlateDecode";
        let ctm = Transform2D::new(50.0, 0.0, 0.0, 50.0, 10.0, 10.0);

        let plan = plan_image_decode(&meta, &ctm, &vp, base, false);
        let key_str = plan.cache_key.to_cache_string();

        // Verify the key contains the source region and reduction markers.
        assert!(key_str.contains(":codec:lossless:"), "{key_str}");
        assert!(key_str.contains(":sr:full:"), "{key_str}");
        assert!(key_str.contains(":rl:none"), "{key_str}");
        assert!(key_str.contains(":comp:all:"), "{key_str}");
        assert!(
            !key_str.contains("SubRect") && !key_str.contains("PowerOfTwo"),
            "cache key must not depend on Rust Debug enum formatting: {key_str}"
        );
    }

    #[test]
    fn cache_key_uses_stable_subrect_source_region_fragment() {
        let vp = test_viewport();
        let base = "xobject:5:0:50:50:1:CCITTFaxDecode";
        let ctm = Transform2D::new(150.0, 0.0, 0.0, 150.0, -25.0, -25.0);
        let mut meta = metadata_for_test();
        meta.width = 50;
        meta.height = 50;
        meta.bits_per_component = 1;
        meta.color_space = "DeviceGray".to_string();
        meta.filters = vec!["CCITTFaxDecode".to_string()];

        let plan = plan_image_decode(&meta, &ctm, &vp, base, false);
        let key_str = plan.cache_key.to_cache_string();

        assert!(key_str.contains(":codec:ccitt:"), "{key_str}");
        assert!(key_str.contains(":sr:subrect:"), "{key_str}");
        assert!(key_str.contains(":rl:none"), "{key_str}");
        assert!(key_str.contains(":comp:all:"), "{key_str}");
        assert!(
            !key_str.contains("SubRect") && !key_str.contains("PowerOfTwo"),
            "cache key must not depend on Rust Debug enum formatting: {key_str}"
        );
    }

    #[test]
    fn cache_key_includes_component_selection_identity() {
        let vp = test_viewport();
        let base = "xobject:5:0:200:200:8:JPXDecode";
        let ctm = Transform2D::new(50.0, 0.0, 0.0, 50.0, 10.0, 10.0);
        let mut meta = metadata_for_test();
        meta.filters = vec!["JPXDecode".to_string()];
        let baseline = plan_image_decode(&meta, &ctm, &vp, base, false)
            .cache_key
            .to_cache_string();

        let mut component_subset = meta;
        component_subset.component_selection = ImageComponentSelection::Components(vec![0, 2]);
        let subset_plan = plan_image_decode(&component_subset, &ctm, &vp, base, false);
        let subset_key = subset_plan.cache_key.to_cache_string();

        assert_ne!(
            baseline, subset_key,
            "cache keys must not collide between all-component and component-subset JPX plans"
        );
        assert!(subset_plan.requires_component_decode);
        assert_eq!(
            subset_plan.capability_report.component_decode,
            ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::ComponentSelectionUnavailable
            )
        );
        assert!(subset_key.contains(":comp:components:0:2:"), "{subset_key}");
    }

    #[test]
    fn cache_key_includes_target_format_and_backend_identity() {
        let vp = test_viewport();
        let base = "xobject:5:0:200:200:8:FlateDecode";
        let ctm = Transform2D::new(50.0, 0.0, 0.0, 50.0, 10.0, 10.0);
        let meta = metadata_for_test();

        let baseline = plan_image_decode(&meta, &ctm, &vp, base, false)
            .cache_key
            .to_cache_string();
        assert!(
            baseline.contains(":fmt:raw-image-8-interleaved:backend:standard-cpu:"),
            "{baseline}"
        );

        let scalar_plan = plan_image_decode_with_identity(
            &meta,
            &ctm,
            &vp,
            base,
            false,
            ImageDecodePlanIdentity {
                target_format: ImageDecodeTargetFormat::RawImage8Interleaved,
                backend: ImageDecodeBackendIdentity::from(BackendSelection::ScalarReference),
                render_contract_fingerprint: "test-default-render-contract".to_string(),
            },
        );
        let scalar_key = scalar_plan.cache_key.to_cache_string();
        assert_ne!(
            baseline, scalar_key,
            "cache keys must not collide between StandardCpu and ScalarReference decode output"
        );
        assert!(
            scalar_key.contains(":fmt:raw-image-8-interleaved:backend:scalar-reference:"),
            "{scalar_key}"
        );

        let hybrid_plan = plan_image_decode_with_identity(
            &meta,
            &ctm,
            &vp,
            base,
            false,
            ImageDecodePlanIdentity::for_backend(
                BackendSelection::ResearchHybrid,
                "test-default-render-contract",
            ),
        );
        let hybrid_key = hybrid_plan.cache_key.to_cache_string();
        assert_ne!(
            baseline, hybrid_key,
            "cache keys must not collide between StandardCpu and ResearchHybrid decode output"
        );
        assert_ne!(
            scalar_key, hybrid_key,
            "cache keys must not collide between backend identities"
        );
        assert!(
            hybrid_key.contains(":fmt:raw-image-8-interleaved:backend:research-hybrid:"),
            "{hybrid_key}"
        );
    }

    #[test]
    fn component_subset_plan_fails_exact_when_native_component_decode_is_unavailable() {
        let vp = Viewport::new([0.0, 0.0, 300.0, 300.0], 72);
        let base = "xobject:5:0:200:200:8:JPXDecode";
        let ctm = Transform2D::new(200.0, 0.0, 0.0, 200.0, 20.0, 20.0);
        let mut meta = metadata_for_test();
        meta.filters = vec!["JPXDecode".to_string()];
        meta.component_selection = ImageComponentSelection::Components(vec![0, 2]);

        let plan = plan_image_decode(&meta, &ctm, &vp, base, true);

        assert_eq!(plan.decision, ImageDecodePlanDecision::DecodeRequired);
        assert!(!plan.requires_source_region_decode);
        assert!(!plan.requires_reduction_decode);
        assert!(plan.requires_component_decode);
        assert_eq!(
            plan.capability_report.component_decode,
            ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::ComponentSelectionUnavailable
            )
        );
        assert!(plan.requires_unavailable_exact_decode_support());
        assert_eq!(
            plan.exact_decode_limitation_summary(),
            "component-selection"
        );
    }

    #[test]
    fn raw_component_subset_plan_uses_native_component_decode() {
        let vp = Viewport::new([0.0, 0.0, 300.0, 300.0], 72);
        let base = "xobject:5:0:200:200:8:raw";
        let ctm = Transform2D::new(200.0, 0.0, 0.0, 200.0, 20.0, 20.0);
        let mut meta = metadata_for_test();
        meta.filters = vec![];
        meta.component_selection = ImageComponentSelection::Components(vec![0, 2]);

        let plan = plan_image_decode(&meta, &ctm, &vp, base, true);

        assert_eq!(plan.decision, ImageDecodePlanDecision::DecodeRequired);
        assert_eq!(plan.capability_report.codec, ImageDecodeCodec::Raw);
        assert!(!plan.requires_source_region_decode);
        assert!(!plan.requires_reduction_decode);
        assert!(plan.requires_component_decode);
        assert_eq!(
            plan.capability_report.component_decode,
            ImageDecodeCapabilityStatus::Native
        );
        assert!(!plan.requires_unavailable_exact_decode_support());
        assert_eq!(plan.exact_decode_limitation_summary(), "none");
        assert!(plan
            .cache_key
            .to_cache_string()
            .contains(":comp:components:0:2:"));
    }

    #[test]
    fn explicit_all_components_do_not_require_subset_decode() {
        let vp = Viewport::new([0.0, 0.0, 300.0, 300.0], 72);
        let base = "xobject:5:0:200:200:8:JPXDecode";
        let ctm = Transform2D::new(200.0, 0.0, 0.0, 200.0, 20.0, 20.0);
        let mut meta = metadata_for_test();
        meta.filters = vec!["JPXDecode".to_string()];
        meta.component_selection = ImageComponentSelection::Components(vec![0, 1, 2]);

        let plan = plan_image_decode(&meta, &ctm, &vp, base, true);

        assert_eq!(plan.decision, ImageDecodePlanDecision::DecodeRequired);
        assert_eq!(plan.capability_report.codec, ImageDecodeCodec::Jpx);
        assert!(!plan.requires_component_decode);
        assert!(!plan.requires_unavailable_exact_decode_support());
        assert_eq!(plan.exact_decode_limitation_summary(), "none");
        assert!(plan
            .cache_key
            .to_cache_string()
            .contains(":comp:components:0:1:2:"));
    }

    #[test]
    fn raw_component_subset_plan_rejects_invalid_known_component_index() {
        let vp = Viewport::new([0.0, 0.0, 300.0, 300.0], 72);
        let base = "xobject:5:0:200:200:8:raw";
        let ctm = Transform2D::new(200.0, 0.0, 0.0, 200.0, 20.0, 20.0);
        let mut meta = metadata_for_test();
        meta.filters = vec![];
        meta.component_selection = ImageComponentSelection::Components(vec![0, 3]);

        let plan = plan_image_decode(&meta, &ctm, &vp, base, true);

        assert_eq!(plan.decision, ImageDecodePlanDecision::DecodeRequired);
        assert_eq!(plan.capability_report.codec, ImageDecodeCodec::Raw);
        assert!(plan.requires_component_decode);
        assert_eq!(
            plan.capability_report.component_decode,
            ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::ComponentSelectionUnavailable
            )
        );
        assert!(plan.requires_unavailable_exact_decode_support());
        assert_eq!(
            plan.exact_decode_limitation_summary(),
            "component-selection"
        );
    }

    #[test]
    fn ccitt_single_component_selection_is_trivial_native() {
        let vp = Viewport::new([0.0, 0.0, 300.0, 300.0], 72);
        let base = "xobject:5:0:200:200:1:CCITTFaxDecode";
        let ctm = Transform2D::new(200.0, 0.0, 0.0, 200.0, 20.0, 20.0);
        let mut meta = metadata_for_test();
        meta.width = 200;
        meta.height = 200;
        meta.bits_per_component = 1;
        meta.color_space = "DeviceGray".to_string();
        meta.filters = vec!["CCITTFaxDecode".to_string()];
        meta.component_selection = ImageComponentSelection::Components(vec![0]);

        let plan = plan_image_decode(&meta, &ctm, &vp, base, true);

        assert_eq!(plan.decision, ImageDecodePlanDecision::DecodeRequired);
        assert_eq!(plan.capability_report.codec, ImageDecodeCodec::Ccitt);
        assert!(!plan.requires_component_decode);
        assert_eq!(
            plan.capability_report.component_decode,
            ImageDecodeCapabilityStatus::Native
        );
        assert!(!plan.requires_unavailable_exact_decode_support());
        assert_eq!(plan.exact_decode_limitation_summary(), "none");
        assert!(plan
            .cache_key
            .to_cache_string()
            .contains(":comp:components:0:"));
    }

    #[test]
    fn ccitt_component_selection_rejects_invalid_known_component_index() {
        let vp = Viewport::new([0.0, 0.0, 300.0, 300.0], 72);
        let base = "xobject:5:0:200:200:1:CCITTFaxDecode";
        let ctm = Transform2D::new(200.0, 0.0, 0.0, 200.0, 20.0, 20.0);
        let mut meta = metadata_for_test();
        meta.width = 200;
        meta.height = 200;
        meta.bits_per_component = 1;
        meta.color_space = "DeviceGray".to_string();
        meta.filters = vec!["CCITTFaxDecode".to_string()];
        meta.component_selection = ImageComponentSelection::Components(vec![1]);

        let plan = plan_image_decode(&meta, &ctm, &vp, base, true);

        assert_eq!(plan.decision, ImageDecodePlanDecision::DecodeRequired);
        assert_eq!(plan.capability_report.codec, ImageDecodeCodec::Ccitt);
        assert!(plan.requires_component_decode);
        assert_eq!(
            plan.capability_report.component_decode,
            ImageDecodeCapabilityStatus::Unavailable(
                ImageDecodeUnavailableReason::ComponentSelectionUnavailable
            )
        );
        assert!(plan.requires_unavailable_exact_decode_support());
        assert_eq!(
            plan.exact_decode_limitation_summary(),
            "component-selection"
        );
    }

    #[test]
    fn image_reference_capability_reports_shape_aware_raw_and_ccitt_windows() {
        let raw = ImageReference {
            page_number: 1,
            xobject_name: "Raw".to_string(),
            object_number: 7,
            generation_number: 0,
            width: 8,
            height: 8,
            bits_per_component: 8,
            color_space: "DeviceRGB".to_string(),
            filter: vec![],
            is_inline: false,
            is_mask: false,
            is_smask: false,
            inline_data: None,
        };
        let raw_capability = image_decode_capabilities_for_image_reference(&raw);
        assert_eq!(raw_capability.codec, ImageDecodeCodec::Raw);
        assert_eq!(
            raw_capability.region_decode,
            ImageDecodeCapabilityStatus::Native
        );
        assert_eq!(
            raw_capability.component_decode,
            ImageDecodeCapabilityStatus::Native
        );

        let ccitt = ImageReference {
            page_number: 1,
            xobject_name: "Fax".to_string(),
            object_number: 8,
            generation_number: 0,
            width: 8,
            height: 8,
            bits_per_component: 1,
            color_space: "DeviceGray".to_string(),
            filter: vec!["CCITTFaxDecode".to_string()],
            is_inline: false,
            is_mask: false,
            is_smask: false,
            inline_data: None,
        };
        let ccitt_capability = image_decode_capabilities_for_image_reference(&ccitt);
        assert_eq!(ccitt_capability.codec, ImageDecodeCodec::Ccitt);
        assert_eq!(
            ccitt_capability.region_decode,
            ImageDecodeCapabilityStatus::Native
        );
        assert_eq!(
            ccitt_capability.component_decode,
            ImageDecodeCapabilityStatus::Native
        );
    }

    #[test]
    fn cache_key_includes_decode_params_mask_smask_and_interpolation_identity() {
        let vp = test_viewport();
        let base = "xobject:5:0:200:200:8:FlateDecode";
        let ctm = Transform2D::new(50.0, 0.0, 0.0, 50.0, 10.0, 10.0);
        let mut meta = metadata_for_test();
        meta.decode_fingerprint = "hdecode".to_string();
        meta.decode_params_fingerprint = "hdecodeparms".to_string();
        meta.image_mask_fingerprint = "hmask".to_string();
        meta.soft_mask_fingerprint = "hsmask".to_string();
        meta.interpolate = true;

        let plan = plan_image_decode(&meta, &ctm, &vp, base, false);
        let key_str = plan.cache_key.to_cache_string();

        assert!(key_str.contains(":dec:hdecode:"), "{key_str}");
        assert!(key_str.contains(":dparms:hdecodeparms:"), "{key_str}");
        assert!(key_str.contains(":im:hmask:"), "{key_str}");
        assert!(key_str.contains(":sm:hsmask:"), "{key_str}");
        assert!(key_str.contains(":interp:true"), "{key_str}");
    }

    #[test]
    fn cache_key_differs_for_decode_params_mask_smask_and_interpolation_changes() {
        let vp = test_viewport();
        let base = "xobject:5:0:200:200:8:FlateDecode";
        let ctm = Transform2D::new(50.0, 0.0, 0.0, 50.0, 10.0, 10.0);
        let meta = metadata_for_test();
        let baseline = plan_image_decode(&meta, &ctm, &vp, base, false)
            .cache_key
            .to_cache_string();

        let mut decode_changed = meta.clone();
        decode_changed.decode_fingerprint = "hdecode".to_string();
        assert_ne!(
            baseline,
            plan_image_decode(&decode_changed, &ctm, &vp, base, false)
                .cache_key
                .to_cache_string()
        );

        let mut decode_params_changed = meta.clone();
        decode_params_changed.decode_params_fingerprint = "hdecodeparms".to_string();
        assert_ne!(
            baseline,
            plan_image_decode(&decode_params_changed, &ctm, &vp, base, false)
                .cache_key
                .to_cache_string()
        );

        let mut mask_changed = meta.clone();
        mask_changed.image_mask_fingerprint = "hmask".to_string();
        assert_ne!(
            baseline,
            plan_image_decode(&mask_changed, &ctm, &vp, base, false)
                .cache_key
                .to_cache_string()
        );

        let mut smask_changed = meta.clone();
        smask_changed.soft_mask_fingerprint = "hsmask".to_string();
        assert_ne!(
            baseline,
            plan_image_decode(&smask_changed, &ctm, &vp, base, false)
                .cache_key
                .to_cache_string()
        );

        let mut interpolation_changed = meta;
        interpolation_changed.interpolate = true;
        assert_ne!(
            baseline,
            plan_image_decode(&interpolation_changed, &ctm, &vp, base, false)
                .cache_key
                .to_cache_string()
        );
    }

    #[test]
    fn degenerate_ctm_fails_open_allows_decode() {
        let vp = test_viewport();
        let meta = metadata_for_test();
        let base = "xobject:5:0:200:200:8:FlateDecode";
        // Zero CTM — degenerate, produces no valid bounds
        let ctm = Transform2D::new(0.0, 0.0, 0.0, 0.0, 50.0, 50.0);

        let plan = plan_image_decode(&meta, &ctm, &vp, base, false);
        // Degenerate CTM should fail open (allow decode) rather than incorrectly
        // cull the image.
        assert_eq!(plan.decision, ImageDecodePlanDecision::DecodeRequired);
    }

    #[test]
    fn non_axis_aligned_ctm_gives_zero_target_dimensions() {
        let vp = test_viewport();
        let meta = metadata_for_test();
        let base = "xobject:5:0:200:200:8:FlateDecode";
        // Rotated CTM (b != 0, c != 0)
        let ctm = Transform2D::new(30.0, 20.0, -20.0, 30.0, 10.0, 10.0);

        let plan = plan_image_decode(&meta, &ctm, &vp, base, false);
        assert_eq!(plan.cache_key.contract.target_width, 0);
        assert_eq!(plan.cache_key.contract.target_height, 0);
        // Still requires decode since it intersects
        assert_eq!(plan.decision, ImageDecodePlanDecision::DecodeRequired);
    }

    #[test]
    fn invisible_image_does_not_produce_cache_entry() {
        // This test verifies the contract: when plan_image_decode returns
        // SkipOutsideViewport, the caller knows NOT to invoke the decoder
        // or insert anything into the cache.
        let vp = small_tile_viewport(); // 50x50 at origin
                                        // Image entirely at x=[80..100], outside the viewport
        let ctm = Transform2D::new(20.0, 0.0, 0.0, 20.0, 80.0, 10.0);
        let meta = metadata_for_test();
        let plan = plan_image_decode(&meta, &ctm, &vp, "test:5:0:200:200:8:FlateDecode", false);

        assert_eq!(
            plan.decision,
            ImageDecodePlanDecision::SkipOutsideViewport,
            "image at x=[80..100] must be outside 50px-wide viewport"
        );
        // In the real renderer, this decision means scheduled_decode_image
        // is never called, so no cache entry is created and no decode work
        // is performed.
    }

    #[test]
    fn tile_origin_participates_in_metadata_culling() {
        let full = Viewport::new([0.0, 0.0, 200.0, 200.0], 72);
        let right_tile = full.pixel_window(100, 0, 100, 200);
        let meta = metadata_for_test();
        let base = "xobject:5:0:200:200:8:FlateDecode";

        let left_image = Transform2D::new(30.0, 0.0, 0.0, 30.0, 10.0, 80.0);
        let left_plan = plan_image_decode(&meta, &left_image, &right_tile, base, false);
        assert_eq!(
            left_plan.decision,
            ImageDecodePlanDecision::SkipOutsideViewport
        );

        let right_image = Transform2D::new(30.0, 0.0, 0.0, 30.0, 120.0, 80.0);
        let right_plan = plan_image_decode(&meta, &right_image, &right_tile, base, false);
        assert_eq!(right_plan.decision, ImageDecodePlanDecision::DecodeRequired);
    }

    #[test]
    fn same_image_different_transform_produces_different_cache_keys() {
        let vp = test_viewport();
        let meta = metadata_for_test();
        let base = "xobject:5:0:200:200:8:FlateDecode";

        let ctm_a = Transform2D::new(40.0, 0.0, 0.0, 40.0, 5.0, 5.0);
        let ctm_b = Transform2D::new(60.0, 0.0, 0.0, 60.0, 5.0, 5.0);

        let plan_a = plan_image_decode(&meta, &ctm_a, &vp, base, false);
        let plan_b = plan_image_decode(&meta, &ctm_b, &vp, base, false);

        assert_ne!(
            plan_a.cache_key.to_cache_string(),
            plan_b.cache_key.to_cache_string(),
            "same image at different scales must have different cache keys"
        );
    }
}
