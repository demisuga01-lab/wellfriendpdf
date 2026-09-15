//! Universal editing v2 orchestration.
//!
//! This module is the additive, binding-safe transaction contract that joins
//! SourceEditing, EditingTransactions, TextReflow, AdvancedEditing, the
//! canonical writer, and renderer invalidation. It deliberately distinguishes
//! an operation that is ready, one that needs an explicit human decision, a
//! policy denial, and input that cannot be recovered safely.

use crate::advanced_editing::{
    analyze_multi_run_text_range, edit_vector_object, list_vector_objects,
    SharedFormEditPolicy, VectorEditOperation, VectorEditOptions, VectorFormInvocation,
};
use crate::content::{ContentToken, ContentTokenizer, SpannedContentToken};
use crate::editing_transactions::{
    apply_scene_text_transaction, build_document_snapshot, build_scene_graph,
    plan_scene_text_transaction, substitution_report_with_source_font, EditableSceneGraph,
    SceneTextEditRequest,
};
use crate::filters::{
    decode_stream_lossless_with_limits, flate_encode_cancellable, DecodeLimits,
    StreamDecodeStatus,
};
use crate::images::decoder::{ImageDecoder, RawImage};
use crate::images::locator::{ImageLocator, ImageReference};
use crate::object::PdfObject;
use crate::secure_mutation::{
    analyze_edit_policy, EditOperation as SignatureEditOperation, EditPolicyDecision,
    EditPolicyReport,
};
use crate::writer::{write_incremental_update, IncrementalObject};
use crate::{ContentEngine, PdfDictionary, Result, WellfriendError};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const UNIVERSAL_EDITING_SCHEMA_VERSION: &str =
    "universal_editing.document-transaction.v2";
const MAX_ANALYSIS_PAGES: usize = 200_000;
const MAX_TEXT_CANDIDATES: usize = 1_000_000;
const MAX_IMAGE_OCCURRENCES: usize = 1_000_000;
const MAX_IMAGE_PIXELS: u64 = 1_000_000_000;
const MAX_IMAGE_PAYLOAD_BYTES: usize = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UniversalPlanStateV2 {
    Ready,
    ApprovalRequired,
    PolicyDenied,
    TargetNotFound,
    IrrecoverableInput,
}

fn promote_plan_state(current: &mut UniversalPlanStateV2, next: UniversalPlanStateV2) {
    fn precedence(state: UniversalPlanStateV2) -> u8 {
        match state {
            UniversalPlanStateV2::Ready => 0,
            UniversalPlanStateV2::ApprovalRequired => 1,
            UniversalPlanStateV2::PolicyDenied => 2,
            UniversalPlanStateV2::TargetNotFound => 3,
            UniversalPlanStateV2::IrrecoverableInput => 4,
        }
    }
    if precedence(next) > precedence(*current) {
        *current = next;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UniversalEditOutcomeV2 {
    Applied,
    ApprovalRequired,
    PolicyDenied,
    TargetNotFound,
    IrrecoverableInput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UniversalMutationModeV2 {
    PreserveSignatures,
    AuthorizedRewrite,
}

impl Default for UniversalMutationModeV2 {
    fn default() -> Self {
        Self::PreserveSignatures
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UniversalAmbiguityPolicyV2 {
    PreviewAndConfirm,
    AutomaticExactOnly,
}

impl Default for UniversalAmbiguityPolicyV2 {
    fn default() -> Self {
        Self::PreviewAndConfirm
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UniversalSharedResourcePolicyV2 {
    CloneOne,
    EditAll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UniversalConformanceProfileV2 {
    PdfA1B,
    PdfA2B,
    PdfA2A,
    PdfA3B,
    PdfA3A,
    PdfUa1,
    PdfX1A2001,
    PdfX3_2003,
    PdfX4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UniversalStandardEncryptionAlgorithmV2 {
    Rc4_128,
    Aes128,
    Aes256,
    Aes256Gcm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UniversalOutputSecurityPolicyV2 {
    Unencrypted,
    Standard {
        algorithm: UniversalStandardEncryptionAlgorithmV2,
        #[serde(default = "default_all_permissions")]
        permissions: i32,
        #[serde(default = "default_true")]
        encrypt_metadata: bool,
    },
}

impl Default for UniversalOutputSecurityPolicyV2 {
    fn default() -> Self {
        Self::Unencrypted
    }
}

fn default_all_permissions() -> i32 {
    -1
}

/// Apply-only secrets. This type deliberately implements neither Serialize nor
/// Debug, so approval plans and diagnostic reports cannot accidentally emit
/// credentials.
pub struct UniversalOutputSecurityCredentialsV2 {
    pub user_password: crate::crypto::SecretBytes,
    pub owner_password: crate::crypto::SecretBytes,
}

impl UniversalConformanceProfileV2 {
    fn label(self) -> &'static str {
        match self {
            Self::PdfA1B => "PDF/A-1B",
            Self::PdfA2B => "PDF/A-2B",
            Self::PdfA2A => "PDF/A-2A",
            Self::PdfA3B => "PDF/A-3B",
            Self::PdfA3A => "PDF/A-3A",
            Self::PdfUa1 => "PDF/UA-1",
            Self::PdfX1A2001 => "PDF/X-1a:2001",
            Self::PdfX3_2003 => "PDF/X-3:2003",
            Self::PdfX4 => "PDF/X-4",
        }
    }
}

impl Default for UniversalSharedResourcePolicyV2 {
    fn default() -> Self {
        Self::CloneOne
    }
}

impl From<UniversalSharedResourcePolicyV2> for SharedFormEditPolicy {
    fn from(value: UniversalSharedResourcePolicyV2) -> Self {
        match value {
            UniversalSharedResourcePolicyV2::CloneOne => Self::CloneEditOneInstance,
            UniversalSharedResourcePolicyV2::EditAll => Self::EditAllUses,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UniversalCapabilityStatusV2 {
    Implemented,
    ApprovalRequired,
    PolicyLimited,
    IrrecoverableOnly,
    Unverified,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniversalCapabilityV2 {
    pub id: String,
    pub status: UniversalCapabilityStatusV2,
    pub owner: String,
    pub behavior: String,
    pub approval_triggers: Vec<String>,
    pub policy_limits: Vec<String>,
    pub source_implementation: String,
    pub qualification_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniversalAnalyzeOptionsV2 {
    #[serde(default)]
    pub pages: Vec<usize>,
    #[serde(default = "default_analysis_page_limit")]
    pub max_pages: usize,
    #[serde(default)]
    pub include_capabilities: bool,
}

impl Default for UniversalAnalyzeOptionsV2 {
    fn default() -> Self {
        Self {
            pages: Vec::new(),
            max_pages: default_analysis_page_limit(),
            include_capabilities: true,
        }
    }
}

fn default_analysis_page_limit() -> usize {
    4_096
}

#[derive(Debug, Clone, Serialize)]
pub struct UniversalDocumentModelV2 {
    pub schema_version: String,
    pub document_id: String,
    pub revision_id: String,
    pub snapshot_id: String,
    pub page_count: usize,
    pub analyzed_pages: Vec<usize>,
    pub graph: EditableSceneGraph,
    pub image_occurrences: Vec<UniversalImageOccurrenceV2>,
    pub capabilities: Vec<UniversalCapabilityV2>,
    pub input_contract: Value,
    pub lazy_analysis: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniversalRenderQualificationOptionsV2 {
    #[serde(default)]
    pub pages: Vec<usize>,
    #[serde(default = "default_render_qualification_dpi")]
    pub dpi: u32,
    #[serde(default = "default_true")]
    pub require_exact: bool,
    #[serde(default = "default_analysis_page_limit")]
    pub max_pages: usize,
    /// Optional independent RGBA oracles keyed by 1-based page number.  Raw
    /// pixels keep this core API decoder-independent and make the comparison
    /// reproducible across bindings and hosts.
    #[serde(default)]
    pub reference_rasters: Vec<UniversalReferenceRasterV2>,
    /// A second request-local guard in addition to the renderer's global pixel
    /// budget.  Zero selects the conservative default.
    #[serde(default)]
    pub max_total_pixels: u64,
}

impl Default for UniversalRenderQualificationOptionsV2 {
    fn default() -> Self {
        Self {
            pages: Vec::new(),
            dpi: default_render_qualification_dpi(),
            require_exact: true,
            max_pages: default_analysis_page_limit(),
            reference_rasters: Vec::new(),
            max_total_pixels: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniversalReferenceRasterV2 {
    pub page: usize,
    pub width: u32,
    pub height: u32,
    /// Straight-alpha, row-major RGBA8 bytes with no row padding.
    pub rgba: Vec<u8>,
    #[serde(default)]
    pub max_mean_absolute_error: f64,
    #[serde(default = "default_reference_min_ssim")]
    pub min_ssim: f64,
    #[serde(default)]
    pub max_channel_error: u8,
}

fn default_reference_min_ssim() -> f64 {
    1.0
}

fn default_render_qualification_pixel_limit() -> u64 {
    1_000_000_000
}

fn default_render_qualification_dpi() -> u32 {
    144
}

fn default_true() -> bool {
    true
}

/// Compile the retained plan, render native pixels, and optionally compare
/// those pixels with an independently supplied reference raster.  A native-only
/// pass proves execution, not cross-renderer equivalence; a reference pass is
/// reported separately with explicit numerical thresholds.
pub fn qualify_universal_render_v2(
    input: &[u8],
    options: &UniversalRenderQualificationOptionsV2,
) -> Result<Value> {
    crate::cancel::check_current_cancel("universal render qualification setup")?;
    if !(24..=2400).contains(&options.dpi) {
        return Err(WellfriendError::invalid_input(
            "universal render qualification dpi must be in 24..=2400",
        ));
    }
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let page_count = engine.page_count()?;
    let pages = normalize_pages(
        &options.pages,
        page_count,
        options.max_pages.clamp(1, MAX_ANALYSIS_PAGES),
    )?;
    let mode = if options.require_exact {
        crate::render::RenderMode::HighQuality
    } else {
        crate::render::RenderMode::Compat
    };
    let pixel_limit = if options.max_total_pixels == 0 {
        default_render_qualification_pixel_limit()
    } else {
        options.max_total_pixels
    };
    if pixel_limit > default_render_qualification_pixel_limit() {
        return Err(WellfriendError::ResourceLimit(format!(
            "universal render qualification max_total_pixels {pixel_limit} exceeds {}",
            default_render_qualification_pixel_limit()
        )));
    }
    let reference_pages = options
        .reference_rasters
        .iter()
        .map(|reference| reference.page)
        .collect::<BTreeSet<_>>();
    if reference_pages.len() != options.reference_rasters.len() {
        return Err(WellfriendError::invalid_input(
            "universal render qualification reference_rasters contains duplicate pages",
        ));
    }
    if reference_pages
        .iter()
        .any(|page| !pages.contains(page))
    {
        return Err(WellfriendError::invalid_input(
            "universal render qualification reference page is outside the selected page set",
        ));
    }
    let mut all_source_plans_eligible = true;
    let mut all_reference_comparisons_passed = true;
    let mut total_pixels = 0u64;
    let mut page_reports = Vec::with_capacity(pages.len());
    let cancel = crate::cancel::current_cancel_token();
    for page in pages {
        cancel.check("universal render qualification page")?;
        let mut contract = engine.default_render_contract(page, options.dpi, mode)?;
        contract.exactness = if options.require_exact {
            crate::render::ExactnessPolicy::HighQualityExact
        } else {
            crate::render::ExactnessPolicy::Compatibility
        };
        let planned_pixels = u64::from(contract.width)
            .checked_mul(u64::from(contract.height))
            .ok_or_else(|| {
                WellfriendError::ResourceLimit(
                    "universal render qualification planned dimensions overflow".to_string(),
                )
            })?;
        let planned_total = total_pixels.checked_add(planned_pixels).ok_or_else(|| {
            WellfriendError::ResourceLimit(
                "universal render qualification planned total pixel count overflow".to_string(),
            )
        })?;
        if planned_total > pixel_limit {
            return Err(WellfriendError::ResourceLimit(format!(
                "universal render qualification requires {planned_total} pixels; request limit is {pixel_limit}"
            )));
        }
        let list = engine.build_page_display_list(page, options.dpi)?;
        let fully_supported = list.is_fully_supported();
        let unsupported = list
            .unsupported
            .iter()
            .map(|operation| {
                json!({
                    "operator": operation.operator,
                    "reason": operation.reason,
                    "silent_fallback": false,
                })
            })
            .collect::<Vec<_>>();
        let compile = match engine.compile_render_plan(contract.clone()) {
            Ok(plan) => json!({
                "status": "compiled",
                "contract_cache_fingerprint": contract.cache_fingerprint(),
                "hot_operation_count": plan.packed.hot_ops.len(),
                "path_count": plan.packed.paths.len(),
                "descriptor_count": plan.packed.descriptors.len(),
                "compile_diagnostics": plan.packed.cold.diagnostics.clone(),
                "batch_count": plan.batches.len(),
            }),
            Err(error) => {
                all_source_plans_eligible = false;
                json!({"status": "refused", "reason": error.to_string(), "silent_fallback": false})
            }
        };
        if !fully_supported {
            all_source_plans_eligible = false;
        }
        let pixels = engine.render_page_cancellable_with_mode(page, options.dpi, &cancel, mode)?;
        let rendered_pixels = u64::from(pixels.width)
            .checked_mul(u64::from(pixels.height))
            .ok_or_else(|| {
                WellfriendError::ResourceLimit(
                    "universal render qualification pixel dimensions overflow".to_string(),
                )
            })?;
        if pixels.width != contract.width
            || pixels.height != contract.height
            || rendered_pixels != planned_pixels
        {
            return Err(WellfriendError::MalformedPdf(format!(
                "universal render qualification returned {}x{} pixels for planned {}x{} output",
                pixels.width, pixels.height, contract.width, contract.height
            )));
        }
        total_pixels = planned_total;
        let rgba = pixels.rgba_bytes();
        let pixel_sha256 = digest_hex_cancellable(
            rgba,
            "universal render qualification native raster digest",
        )?;
        let reference_comparison = if let Some(reference) = options
            .reference_rasters
            .iter()
            .find(|reference| reference.page == page)
        {
            let comparison = compare_reference_raster_v2(
                pixels.width,
                pixels.height,
                rgba,
                reference,
            )?;
            all_reference_comparisons_passed &= comparison["passed"] == Value::Bool(true);
            comparison
        } else {
            json!({"status": "not_supplied", "passed": Value::Null})
        };
        page_reports.push(json!({
            "page": page,
            "contract": contract,
            "display_list_fully_supported": fully_supported,
            "unsupported_operations": unsupported,
            "stats": {
                "operations": list.stats.operations,
                "paths": list.stats.paths,
                "text_ops": list.stats.text_ops,
                "image_xobjects": list.stats.image_xobjects,
                "inline_images": list.stats.inline_images,
                "form_xobjects": list.stats.form_xobjects,
                "shadings": list.stats.shadings,
                "patterns": list.stats.patterns,
                "transparency_ops": list.stats.transparency_ops,
                "unsupported_ops": list.stats.unsupported_ops,
                "max_stack_depth": list.stats.max_stack_depth,
            },
            "retained_plan": compile,
            "pixel_render": {
                "executed": true,
                "width": pixels.width,
                "height": pixels.height,
                "format": "rgba8_straight_alpha_tightly_packed",
                "byte_length": rgba.len(),
                "sha256": pixel_sha256,
            },
            "reference_comparison": reference_comparison,
        }));
    }
    let every_selected_page_has_reference = reference_pages.len() == page_reports.len();
    let status = if !all_source_plans_eligible {
        "rendered_with_source_plan_refusal_or_incomplete_operation"
    } else if every_selected_page_has_reference && all_reference_comparisons_passed {
        "native_and_supplied_reference_qualification_passed"
    } else if every_selected_page_has_reference {
        "supplied_reference_qualification_failed"
    } else {
        "native_pixels_rendered_reference_corpus_pending"
    };
    Ok(json!({
        "schema_version": UNIVERSAL_EDITING_SCHEMA_VERSION,
        "kind": "universal_render_qualification_v2",
        "status": status,
        "require_exact": options.require_exact,
        "dpi": options.dpi,
        "pages": page_reports,
        "pixel_render_executed": true,
        "total_pixels": total_pixels,
        "all_selected_pages_have_reference": every_selected_page_has_reference,
        "all_reference_comparisons_passed": every_selected_page_has_reference && all_reference_comparisons_passed,
        "font_substitution": "runtime telemetry mandatory; any event is a disclosed visual-equivalence risk",
        "color_management": "contract-selected backend and fallback posture remain in cache identity",
        "silent_fallback_allowed": false,
        "adobe_level_claim": false,
        "qualification_pending": if every_selected_page_has_reference { json!(["VPS corpus breadth", "fuzz and malformed corpus", "PDF/A PDF/UA PDF/X external validators"]) } else { json!(["independent reference renderer rasters", "VPS corpus breadth", "fuzz and malformed corpus", "PDF/A PDF/UA PDF/X external validators"]) },
    }))
}

fn compare_reference_raster_v2(
    width: u32,
    height: u32,
    actual: &[u8],
    reference: &UniversalReferenceRasterV2,
) -> Result<Value> {
    if width == 0 || height == 0 {
        return Err(WellfriendError::MalformedPdf(
            "native renderer returned a zero-sized raster".to_string(),
        ));
    }
    if reference.width != width || reference.height != height {
        return Err(WellfriendError::invalid_input(format!(
            "reference raster for page {} is {}x{} but native render is {width}x{height}",
            reference.page, reference.width, reference.height
        )));
    }
    if !reference.max_mean_absolute_error.is_finite()
        || reference.max_mean_absolute_error < 0.0
        || reference.max_mean_absolute_error > 255.0
        || !reference.min_ssim.is_finite()
        || !(0.0..=1.0).contains(&reference.min_ssim)
    {
        return Err(WellfriendError::invalid_input(
            "reference raster thresholds must be finite with MAE in 0..=255 and SSIM in 0..=1",
        ));
    }
    let expected_len = usize::try_from(width)
        .ok()
        .and_then(|width| usize::try_from(height).ok().and_then(|height| width.checked_mul(height)))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| {
            WellfriendError::ResourceLimit(
                "reference raster byte length overflow".to_string(),
            )
        })?;
    if actual.len() != expected_len || reference.rgba.len() != expected_len {
        return Err(WellfriendError::invalid_input(format!(
            "reference raster for page {} must contain exactly {expected_len} RGBA bytes",
            reference.page
        )));
    }
    let mut absolute_sum = 0u128;
    let mut squared_sum = 0f64;
    let mut maximum = 0u8;
    let mut alpha_absolute_sum = 0u128;
    let mut maximum_alpha_error = 0u8;
    let mut exact_pixels = 0u64;
    let mut luma_actual_sum = 0f64;
    let mut luma_reference_sum = 0f64;
    let pixel_count = u64::from(width) * u64::from(height);
    for (pixel_index, (actual_pixel, reference_pixel)) in actual
        .chunks_exact(4)
        .zip(reference.rgba.chunks_exact(4))
        .enumerate()
    {
        if pixel_index % 65_536 == 0 {
            crate::cancel::check_current_cancel(
                "universal render qualification reference comparison",
            )?;
        }
        let actual_visual = rgba_visual_channels(actual_pixel);
        let reference_visual = rgba_visual_channels(reference_pixel);
        let mut pixel_exact = true;
        for channel in 0..4 {
            let delta = actual_visual[channel].abs_diff(reference_visual[channel]);
            maximum = maximum.max(delta);
            absolute_sum += u128::from(delta);
            squared_sum += f64::from(delta) * f64::from(delta);
            pixel_exact &= delta == 0;
            if channel == 3 {
                alpha_absolute_sum += u128::from(delta);
                maximum_alpha_error = maximum_alpha_error.max(delta);
            }
        }
        exact_pixels += u64::from(pixel_exact);
        luma_actual_sum += rgba_luma(actual_pixel);
        luma_reference_sum += rgba_luma(reference_pixel);
    }
    let sample_count = expected_len as f64;
    let mean_absolute_error = absolute_sum as f64 / sample_count;
    let root_mean_square_error = (squared_sum / sample_count).sqrt();
    let mean_actual = luma_actual_sum / pixel_count as f64;
    let mean_reference = luma_reference_sum / pixel_count as f64;
    let mut variance_actual = 0f64;
    let mut variance_reference = 0f64;
    let mut covariance = 0f64;
    for (pixel_index, (actual_pixel, reference_pixel)) in actual
        .chunks_exact(4)
        .zip(reference.rgba.chunks_exact(4))
        .enumerate()
    {
        if pixel_index % 65_536 == 0 {
            crate::cancel::check_current_cancel(
                "universal render qualification SSIM statistics",
            )?;
        }
        let actual_delta = rgba_luma(actual_pixel) - mean_actual;
        let reference_delta = rgba_luma(reference_pixel) - mean_reference;
        variance_actual += actual_delta * actual_delta;
        variance_reference += reference_delta * reference_delta;
        covariance += actual_delta * reference_delta;
    }
    let divisor = pixel_count.saturating_sub(1).max(1) as f64;
    variance_actual /= divisor;
    variance_reference /= divisor;
    covariance /= divisor;
    let c1 = (0.01_f64 * 255.0).powi(2);
    let c2 = (0.03_f64 * 255.0).powi(2);
    let global_ssim = (((2.0 * mean_actual * mean_reference) + c1) * ((2.0 * covariance) + c2)
        / (((mean_actual * mean_actual) + (mean_reference * mean_reference) + c1)
            * (variance_actual + variance_reference + c2)))
        .clamp(0.0, 1.0);
    let windowed_ssim =
        windowed_luminance_ssim_v2(width, height, actual, &reference.rgba)?;
    let passed = mean_absolute_error <= reference.max_mean_absolute_error
        && maximum <= reference.max_channel_error
        && windowed_ssim >= reference.min_ssim;
    Ok(json!({
        "status": if passed { "passed" } else { "failed" },
        "passed": passed,
        "reference_sha256": digest_hex_cancellable(
            &reference.rgba,
            "universal render qualification reference raster digest",
        )?,
        "mean_absolute_error": mean_absolute_error,
        "root_mean_square_error": root_mean_square_error,
        "maximum_channel_error": maximum,
        "mean_alpha_error": alpha_absolute_sum as f64 / pixel_count.max(1) as f64,
        "maximum_alpha_error": maximum_alpha_error,
        "exact_pixel_ratio": exact_pixels as f64 / pixel_count.max(1) as f64,
        "windowed_luminance_ssim": windowed_ssim,
        "global_luminance_ssim": global_ssim,
        "comparison_color_policy": "RGB composited over white plus independent alpha; hidden RGB at alpha=0 is ignored",
        "thresholds": {
            "max_mean_absolute_error": reference.max_mean_absolute_error,
            "max_channel_error": reference.max_channel_error,
            "min_ssim": reference.min_ssim,
        }
    }))
}

fn rgba_visual_channels(pixel: &[u8]) -> [u8; 4] {
    let alpha = u32::from(pixel[3]);
    let composite = |channel: u8| {
        let numerator = u32::from(channel) * alpha + 255 * (255 - alpha) + 127;
        (numerator / 255) as u8
    };
    [composite(pixel[0]), composite(pixel[1]), composite(pixel[2]), pixel[3]]
}

/// Local 8x8 box-window SSIM. Non-overlapping windows keep qualification O(N)
/// and bounded-memory while detecting spatially localized regressions that a
/// single global mean/variance can hide. Partial edge windows are weighted by
/// their actual pixel count.
fn windowed_luminance_ssim_v2(
    width: u32,
    height: u32,
    actual: &[u8],
    reference: &[u8],
) -> Result<f64> {
    const WINDOW: usize = 8;
    let width = width as usize;
    let height = height as usize;
    let c1 = (0.01_f64 * 255.0).powi(2);
    let c2 = (0.03_f64 * 255.0).powi(2);
    let mut weighted_ssim = 0.0;
    let mut total_weight = 0usize;
    for y0 in (0..height).step_by(WINDOW) {
        crate::cancel::check_current_cancel(
            "universal render qualification windowed SSIM row",
        )?;
        for x0 in (0..width).step_by(WINDOW) {
            let y1 = (y0 + WINDOW).min(height);
            let x1 = (x0 + WINDOW).min(width);
            let mut sum_actual = 0.0;
            let mut sum_reference = 0.0;
            let mut sum_actual_squared = 0.0;
            let mut sum_reference_squared = 0.0;
            let mut sum_cross = 0.0;
            let mut count = 0usize;
            for y in y0..y1 {
                for x in x0..x1 {
                    let offset = (y * width + x) * 4;
                    let actual_luma = rgba_luma(&actual[offset..offset + 4]);
                    let reference_luma = rgba_luma(&reference[offset..offset + 4]);
                    sum_actual += actual_luma;
                    sum_reference += reference_luma;
                    sum_actual_squared += actual_luma * actual_luma;
                    sum_reference_squared += reference_luma * reference_luma;
                    sum_cross += actual_luma * reference_luma;
                    count += 1;
                }
            }
            let count_f64 = count.max(1) as f64;
            let mean_actual = sum_actual / count_f64;
            let mean_reference = sum_reference / count_f64;
            let divisor = count.saturating_sub(1).max(1) as f64;
            let variance_actual =
                ((sum_actual_squared - count_f64 * mean_actual * mean_actual) / divisor).max(0.0);
            let variance_reference = ((sum_reference_squared
                - count_f64 * mean_reference * mean_reference)
                / divisor)
                .max(0.0);
            let covariance =
                (sum_cross - count_f64 * mean_actual * mean_reference) / divisor;
            let ssim = (((2.0 * mean_actual * mean_reference) + c1)
                * ((2.0 * covariance) + c2)
                / (((mean_actual * mean_actual) + (mean_reference * mean_reference) + c1)
                    * (variance_actual + variance_reference + c2)))
                .clamp(0.0, 1.0);
            weighted_ssim += ssim * count_f64;
            total_weight += count;
        }
    }
    Ok((weighted_ssim / total_weight.max(1) as f64).clamp(0.0, 1.0))
}

fn rgba_luma(pixel: &[u8]) -> f64 {
    let alpha = f64::from(pixel[3]) / 255.0;
    let red = f64::from(pixel[0]) * alpha + 255.0 * (1.0 - alpha);
    let green = f64::from(pixel[1]) * alpha + 255.0 * (1.0 - alpha);
    let blue = f64::from(pixel[2]) * alpha + 255.0 * (1.0 - alpha);
    0.2126 * red + 0.7152 * green + 0.0722 * blue
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UniversalImageEncodingV2 {
    RawSamples,
    Flate,
    Jpeg,
    Jpx,
}

/// A bounded PDF color-space graph for replacement image samples.  The legacy
/// `color_space` string remains accepted for DeviceGray/RGB/CMYK; this typed
/// descriptor owns every parameter needed for calibrated, indexed, spot, and
/// n-channel replacement data.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UniversalImageColorSpaceV2 {
    DeviceGray,
    DeviceRgb,
    DeviceCmyk,
    CalGray {
        white_point: [f64; 3],
        #[serde(default)]
        black_point: Option<[f64; 3]>,
        #[serde(default)]
        gamma: Option<f64>,
    },
    CalRgb {
        white_point: [f64; 3],
        #[serde(default)]
        black_point: Option<[f64; 3]>,
        #[serde(default)]
        gamma: Option<[f64; 3]>,
        #[serde(default)]
        matrix: Option<[f64; 9]>,
    },
    Lab {
        white_point: [f64; 3],
        #[serde(default)]
        black_point: Option<[f64; 3]>,
        #[serde(default)]
        range: Option<[f64; 4]>,
    },
    IccBased {
        components: u8,
        profile: Vec<u8>,
        #[serde(default)]
        alternate: Option<Box<UniversalImageColorSpaceV2>>,
        #[serde(default)]
        range: Vec<f64>,
    },
    Indexed {
        base: Box<UniversalImageColorSpaceV2>,
        high_value: u8,
        lookup: Vec<u8>,
    },
    Separation {
        colorant: String,
        alternate: Box<UniversalImageColorSpaceV2>,
        tint_transform: UniversalPdfFunctionV2,
    },
    DeviceN {
        colorants: Vec<String>,
        alternate: Box<UniversalImageColorSpaceV2>,
        tint_transform: UniversalPdfFunctionV2,
        #[serde(default)]
        attributes: BTreeMap<String, UniversalPdfValueV2>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UniversalPdfFunctionV2 {
    Exponential {
        domain: [f64; 2],
        c0: Vec<f64>,
        c1: Vec<f64>,
        exponent: f64,
    },
    Sampled {
        domain: Vec<f64>,
        range: Vec<f64>,
        size: Vec<u32>,
        bits_per_sample: u8,
        samples: Vec<u8>,
        #[serde(default)]
        encode: Vec<f64>,
        #[serde(default)]
        decode: Vec<f64>,
    },
    Stitching {
        domain: [f64; 2],
        #[serde(default)]
        range: Vec<f64>,
        functions: Vec<UniversalPdfFunctionV2>,
        bounds: Vec<f64>,
        encode: Vec<f64>,
    },
    Calculator {
        domain: Vec<f64>,
        range: Vec<f64>,
        program: Vec<u8>,
    },
}

/// Serializable low-level PDF object algebra used by the governed object-graph
/// escape hatch.  It is intentionally complete enough for patterns, shadings,
/// appearance streams, masks, optional-content graphs, and structure trees,
/// while keeping indirect-reference creation revision-bound and deterministic.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UniversalPdfValueV2 {
    Null,
    Boolean { value: bool },
    Integer { value: i64 },
    Real { value: f64 },
    Name { value: String },
    String { value: Vec<u8> },
    Array { items: Vec<UniversalPdfValueV2> },
    Dictionary { entries: BTreeMap<String, UniversalPdfValueV2> },
    Stream {
        entries: BTreeMap<String, UniversalPdfValueV2>,
        data: Vec<u8>,
        #[serde(default)]
        flate_encode: bool,
    },
    Reference { number: u32, #[serde(default)] generation: u16 },
    LocalReference { local_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UniversalObjectTargetV2 {
    Existing {
        number: u32,
        #[serde(default)]
        generation: u16,
        /// SHA-256 of the deterministic debug projection returned by analysis.
        expected_fingerprint: String,
    },
    New { local_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniversalObjectMutationV2 {
    pub target: UniversalObjectTargetV2,
    pub value: UniversalPdfValueV2,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniversalObjectGraphEditRequestV2 {
    pub mutations: Vec<UniversalObjectMutationV2>,
    #[serde(default)]
    pub affected_pages: Vec<usize>,
    /// Required for definitions shared outside the declared page set.
    #[serde(default)]
    pub acknowledge_global_resource_impact: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniversalImageReplacementV2 {
    pub width: u32,
    pub height: u32,
    pub bits_per_component: u8,
    /// Legacy named colour-space spelling. It may be omitted for stencil
    /// image masks or when `color_space_descriptor` is supplied.
    #[serde(default)]
    pub color_space: String,
    #[serde(default)]
    pub color_space_descriptor: Option<UniversalImageColorSpaceV2>,
    pub encoding: UniversalImageEncodingV2,
    pub data: Vec<u8>,
    /// Emit a one-bit stencil image. Stencil images have no colour space;
    /// their samples select where the current non-stroking colour is painted.
    #[serde(default)]
    pub image_mask: bool,
    /// Optional PDF `/Decode` array. Ordinary images require exactly two
    /// finite values per colour component. Stencil images accept only
    /// `[0 1]` or `[1 0]`, making their paint polarity explicit.
    #[serde(default)]
    pub decode: Option<Vec<f64>>,
    #[serde(default)]
    pub preserve_masks: bool,
    /// Optional replacement soft-mask samples. This is independent of the
    /// primary image encoding so decoded RGBA/JPX-alpha occurrences can be
    /// rewritten as a standards-compliant DeviceRGB image plus `/SMask`
    /// instead of mislabelling the fourth channel as CMYK. An explicit soft
    /// mask and `preserve_masks=true` are mutually exclusive.
    #[serde(default)]
    pub soft_mask: Option<UniversalImageSoftMaskV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniversalImageSoftMaskV2 {
    pub width: u32,
    pub height: u32,
    pub bits_per_component: u8,
    /// Uncompressed, row-packed DeviceGray opacity samples.
    pub samples: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniversalImageEditRequestV2 {
    pub page: usize,
    #[serde(default)]
    pub occurrence_id: Option<String>,
    #[serde(default)]
    pub resource_name: Option<String>,
    #[serde(default)]
    pub object_number: Option<u32>,
    #[serde(default)]
    pub generation: u16,
    #[serde(default)]
    pub occurrence_index: usize,
    #[serde(default)]
    pub shared_resource_policy: UniversalSharedResourcePolicyV2,
    pub replacement: UniversalImageReplacementV2,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct UniversalImageMatrixV2 {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl UniversalImageMatrixV2 {
    const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    fn multiply(self, rhs: Self) -> Self {
        let [a, b, c, d, e, f] = crate::content::concat_matrix(
            &[rhs.a, rhs.b, rhs.c, rhs.d, rhs.e, rhs.f],
            &[self.a, self.b, self.c, self.d, self.e, self.f],
        );
        Self { a, b, c, d, e, f }
    }

    fn transform(self, x: f64, y: f64) -> [f64; 2] {
        [
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniversalImageOccurrenceV2 {
    pub occurrence_id: String,
    pub page: usize,
    pub content_stream_index: usize,
    pub owner_stream_object: u32,
    pub owner_stream_generation: u16,
    pub operation_byte_start: usize,
    pub operation_byte_end: usize,
    pub resource_name: Option<String>,
    pub object_number: Option<u32>,
    pub generation: Option<u16>,
    pub inline: bool,
    pub transform: UniversalImageMatrixV2,
    pub bbox: [f64; 4],
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub bits_per_component: Option<u8>,
    pub color_space: Option<String>,
    pub filters: Vec<String>,
    pub invocation_path: Vec<VectorFormInvocation>,
    pub shared_definition_uses: usize,
    pub clone_one_eligible: bool,
    pub edit_all_eligible: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniversalVectorEditRequestV2 {
    pub page: usize,
    pub stable_id: String,
    pub operation: VectorEditOperation,
    #[serde(default)]
    pub shared_resource_policy: UniversalSharedResourcePolicyV2,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniversalStructureCorrectionRequestV2 {
    pub semantic_node_id: String,
    pub text_edit: SceneTextEditRequest,
    #[serde(default)]
    pub accepted_relationships: Vec<String>,
    /// Rebuild the ParentTree and accessibility mutation hooks after the text
    /// source transaction. Enabled by default for this explicitly semantic
    /// operation; callers can disable it only when an ObjectGraph mutation in
    /// the same governed workflow owns a more specific structure update.
    #[serde(default = "default_true")]
    pub repair_tagged_structure: bool,
    #[serde(default)]
    pub structure_language: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UniversalEditOperationV2 {
    Text {
        request: SceneTextEditRequest,
    },
    Image {
        request: UniversalImageEditRequestV2,
    },
    Vector {
        request: UniversalVectorEditRequestV2,
    },
    StructureCorrection {
        request: UniversalStructureCorrectionRequestV2,
    },
    /// Tables, mathematics, OCR/searchable layers, forms, annotations, and XFA
    /// use their existing typed source-owned mutation subsystem under the same
    /// revision/approval transaction.
    DocumentSubsystem {
        request: crate::document_subsystems::DocumentSubsystemsRequest,
    },
    /// Tagged-PDF repair, ParentTree rebuilding, metadata, redaction, and
    /// sanitization use the existing document-security transaction engine.
    DocumentSecurity {
        request: crate::document_security::DocumentSecurityRequest,
    },
    /// Exact indirect-object graph replacement for PDF constructs that have no
    /// safe author-level inverse (patterns, shadings, appearance programs,
    /// masks, optional content, and custom structure extensions).
    ObjectGraph {
        request: UniversalObjectGraphEditRequestV2,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniversalEditPolicyV2 {
    #[serde(default)]
    pub mutation_mode: UniversalMutationModeV2,
    #[serde(default)]
    pub ambiguity: UniversalAmbiguityPolicyV2,
    #[serde(default)]
    pub allow_font_substitution: bool,
    #[serde(default)]
    pub allow_deterministic_repair: bool,
    #[serde(default)]
    pub require_conformance_preservation: bool,
    /// Explicit standards gates executed against the final in-memory bytes
    /// before those bytes are returned to the caller. When the legacy boolean
    /// above is true and this list is empty, profiles declared by the source
    /// document are detected and preserved.
    #[serde(default)]
    pub required_conformance_profiles: Vec<UniversalConformanceProfileV2>,
    #[serde(default)]
    pub output_security: UniversalOutputSecurityPolicyV2,
}

impl Default for UniversalEditPolicyV2 {
    fn default() -> Self {
        Self {
            mutation_mode: UniversalMutationModeV2::PreserveSignatures,
            ambiguity: UniversalAmbiguityPolicyV2::PreviewAndConfirm,
            allow_font_substitution: true,
            allow_deterministic_repair: true,
            require_conformance_preservation: false,
            required_conformance_profiles: Vec::new(),
            output_security: UniversalOutputSecurityPolicyV2::Unencrypted,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniversalEditRequestV2 {
    pub operation: UniversalEditOperationV2,
    #[serde(default)]
    pub policy: UniversalEditPolicyV2,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniversalCandidateV2 {
    pub candidate_id: String,
    pub page: usize,
    pub kind: String,
    pub source_identity: Value,
    pub confidence: f64,
    pub exact: bool,
    pub shared_resource: bool,
    pub approval_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniversalEditPlanV2 {
    pub schema_version: String,
    pub plan_id: String,
    pub document_id: String,
    pub revision_id: String,
    pub snapshot_id: String,
    pub state: UniversalPlanStateV2,
    pub requested_operation: UniversalEditOperationV2,
    pub execution_operation: UniversalEditOperationV2,
    pub policy: UniversalEditPolicyV2,
    pub candidates: Vec<UniversalCandidateV2>,
    pub selected_candidate_ids: Vec<String>,
    pub approval_reasons: Vec<String>,
    pub read_set: Vec<String>,
    pub write_set: Vec<String>,
    pub preview: Value,
    pub signature_impact: Value,
    pub conformance_impact: Value,
    pub implementation_report: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniversalApprovalDecisionV2 {
    #[serde(default)]
    pub selected_candidate_ids: Vec<String>,
    #[serde(default)]
    pub approved_font: Option<String>,
    pub mutation_mode: UniversalMutationModeV2,
    #[serde(default)]
    pub accept_visual_change: bool,
    #[serde(default)]
    pub accept_signature_invalidation: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniversalApprovalTokenV2 {
    pub schema_version: String,
    pub plan_id: String,
    pub revision_id: String,
    pub decision: UniversalApprovalDecisionV2,
    pub binding_digest: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct UniversalEditResultV2 {
    pub schema_version: String,
    pub plan_id: String,
    pub transaction_id: String,
    pub outcome: UniversalEditOutcomeV2,
    pub changed: bool,
    pub input_revision_id: String,
    pub output_revision_id: String,
    pub affected_pages: Vec<usize>,
    pub affected_objects: Vec<String>,
    pub cloned_resources: Vec<String>,
    pub operation_report: Value,
    pub render_invalidation: Value,
    pub signature_impact: Value,
    pub conformance_impact: Value,
    pub inverse: Value,
    pub issues: Vec<Value>,
}

pub fn universal_capability_registry_v2() -> Vec<UniversalCapabilityV2> {
    vec![
        capability("source_text", UniversalCapabilityStatusV2::ApprovalRequired, "SourceEditing/AdvancedEditing", "operator, page-logical partial-token and cross-/Contents multi-run, geometric, and semantic routes remove selected current-revision codes, preserve source advance, shape approved Type0 substitutions, replay per-grapheme paint/text state, and keep clipping or tagged replacements source-inline; direct isomorphic /ActualText carriers and complete selected non-isomorphic /ActualText scopes are neutralized in the same atomic stream transaction so search/copy cannot retain the old value, while legacy single-token writers fail closed on logical-text ownership", &["ambiguous source ownership", "font or layout substitution", "partial non-isomorphic or shared named /ActualText semantic ownership"], &["shared Form-owned text still requires an occurrence-level clone-one/edit-all decision", "vertical inline replacement accepts upright zero-offset glyphs; rotated or GPOS-offset vertical glyphs require an explicit text-matrix snapshot"]),
        capability("image_occurrence", UniversalCapabilityStatusV2::ApprovalRequired, "UniversalEditing/CanonicalWriter", "image definitions can be replaced atomically with Device, calibrated, ICCBased, Indexed, Separation, or DeviceN sample graphs, typed per-component Decode arrays, explicit one-bit stencil semantics, and an explicit same-size DeviceGray soft mask; page image occurrence transforms carry graphics state across ordered /Contents members, inline occurrences can be promoted, nested Forms can be cloned, and decoded RGBA/JPX alpha is never reinterpreted as CMYK", &["shared XObject", "mask resampling", "inline-image promotion"], &["dimension-changing soft-mask resampling requires caller-supplied replacement objects"]),
        capability("vector_graphics", UniversalCapabilityStatusV2::ApprovalRequired, "AdvancedEditing/UniversalObjectGraph", "source-range vector mutation plus governed pattern, shading, appearance, transparency, and optional-content object-graph replacement", &["shared Form occurrence", "global resource impact"], &[]),
        capability("document_reflow", UniversalCapabilityStatusV2::ApprovalRequired, "TextReflow/CanonicalWriter", "constraint-based geometric and semantic reflow can paginate an arbitrarily long bounded replacement across repeated continuation-page chunks at any ordered page boundary while validating every line against the target region, preserving stable page references, and shifting page-label indexes", &["inferred structure", "cross-region movement", "font substitution"], &[]),
        capability("semantic_structure", UniversalCapabilityStatusV2::ApprovalRequired, "TextReflow/DocumentSecurity/UniversalObjectGraph", "revision-bound semantic correction can execute ParentTree repair; exact custom StructTreeRoot and tag-tree objects use the governed object-graph route", &["inferred reading order or relationship"], &[]),
        capability("interactive_content", UniversalCapabilityStatusV2::ApprovalRequired, "DocumentSubsystems/DocumentSecurity/UniversalObjectGraph", "forms, annotations, links, destinations, outlines, labels, XFA, searchable OCR, and visible scanned-word reconstruction are reachable through typed or exact revision-bound operations; visible reconstruction clone-writes one image occurrence after bounded harmonic inpainting, deletes any intersecting pre-existing invisible OCR carrier through its exact page-logical range, and adds shaped embedded Unicode text", &["document action or shared structure ownership changes", "OCR rectangle, exact searchable-layer range when present, and replacement approval"], &["OCR recognition remains provider-owned and inpainting is deterministic reconstruction, not recovery of unknowable original pixels"]),
        capability("signatures", UniversalCapabilityStatusV2::PolicyLimited, "SecureMutation/CanonicalWriter", "preserve-signature and authorized-rewrite modes are distinct", &["authorized rewrite invalidates earlier signatures"], &["credentials and DocMDP permissions remain mandatory"]),
        capability("output_security", UniversalCapabilityStatusV2::PolicyLimited, "CryptoWriter/CanonicalWriter", "authorized full rewrites can emit credentialed AES-256 Standard-handler output through apply-only byte credentials and credentialed reopen", &["security-envelope rotation", "encryption conflicts with PDF/A and PDF/X"], &["output passwords above the ISO 127-byte effective limit are refused instead of silently truncated", "legacy RC4/AES-128 output and PubSec recipient rotation remain separate, explicitly limited routes"]),
        capability("renderer", UniversalCapabilityStatusV2::Unverified, "NativeRenderer", "qualification executes native RGBA rendering, hashes every selected page, discloses unsupported operations, and can compare caller-supplied independent reference rasters with MAE/RMSE/max-error/SSIM gates", &[], &["VPS corpus breadth and independent reference generation remain pending"]),
        capability("request_cancellation", UniversalCapabilityStatusV2::PolicyLimited, "Server/Engine/Filters", "universal HTTP blocking workers install their request token into a panic-safe synchronous scope; analysis, planning, source scans, stream filters, reconstruction, compression, rendering, raster comparison, apply, reopen, and validation poll it cooperatively so timed-out work can release its semaphore permit", &[], &["third-party codec calls that do not expose an interruption callback can stop only at the nearest governed call boundary"]),
        capability("standards", UniversalCapabilityStatusV2::Unverified, "Compliance/StandardsEngine", "requested PDF/A, PDF/UA, and PDF/X profiles are validated against final in-memory bytes and failed/inconclusive output is withheld", &["meaning-dependent accessibility decisions"], &["external accredited certification remains pending"]),
        capability("input_recovery", UniversalCapabilityStatusV2::PolicyLimited, "Parser/CanonicalWriter", "valid PDFs, password-opened Standard-handler encrypted PDFs, and deterministic repair candidates are accepted", &["repair changes object reachability"], &["hostile or irrecoverable byte streams are outside contract", "PubSec universal mutation still requires an explicit retained recipient-provider integration rather than a password credential"]),
        capability("encrypted_documents", UniversalCapabilityStatusV2::PolicyLimited, "UniversalEditing/CryptoWriter", "a plan can bind Standard-handler algorithm, permissions, and metadata policy; apply-only non-serializable credentials drive atomic post-edit re-encryption and credentialed reopen", &["full-rewrite security-envelope rotation", "signature invalidation"], &["Adobe.PubSec recipient-provider rotation remains the explicit PubSec workflow; PDF/A and PDF/X correctly conflict with encryption"]),
        capability("dependency_security", UniversalCapabilityStatusV2::PolicyLimited, "ReleaseGovernance", "release qualification remains fail-closed when an applicable dependency advisory has no patched upstream version", &[], &["RUSTSEC-2023-0071 has no patched rsa release; remotely observable private-key decryption must not be enabled without a separately reviewed native or service-isolated provider"]),
    ]
}

pub fn analyze_universal_document_v2(
    input: &[u8],
    options: &UniversalAnalyzeOptionsV2,
) -> Result<UniversalDocumentModelV2> {
    crate::cancel::check_current_cancel("universal document analysis open")?;
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let page_count = engine.page_count()?;
    let limit = options.max_pages.clamp(1, MAX_ANALYSIS_PAGES);
    let analyzed_pages = normalize_pages(&options.pages, page_count, limit)?;
    crate::cancel::check_current_cancel("universal document scene analysis")?;
    let graph = build_scene_graph(input, &analyzed_pages)?;
    crate::cancel::check_current_cancel("universal document image analysis")?;
    let image_occurrences = universal_image_occurrences_v2(input, &analyzed_pages)?;
    crate::cancel::check_current_cancel("universal document snapshot")?;
    let snapshot = build_document_snapshot(input, None)?;
    Ok(UniversalDocumentModelV2 {
        schema_version: UNIVERSAL_EDITING_SCHEMA_VERSION.to_string(),
        document_id: snapshot.document_id,
        revision_id: snapshot.revision_id,
        snapshot_id: snapshot.snapshot_id,
        page_count,
        analyzed_pages,
        graph,
        image_occurrences,
        capabilities: options
            .include_capabilities
            .then(universal_capability_registry_v2)
            .unwrap_or_default(),
        input_contract: json!({
            "accepted": ["iso_valid_pdf", "deterministically_recoverable_pdf"],
            "observed": universal_input_recovery_state(input),
            "terminal_non_editable": ["policy_denied", "target_not_found", "missing_credentials", "irrecoverable_input", "resource_limit"],
            "ordinary_ambiguity": "approval_required",
        }),
        lazy_analysis: json!({
            "page_windowed": options.pages.is_empty() && page_count > limit,
            "image_candidate_details": "requested_page_window",
            "image_definition_sharing": "document_wide_occurrence_scan_under_global_limit",
            "next_page": analyzed_pages.last().copied().filter(|page| *page < page_count).map(|page| page + 1),
            "max_pages_per_request": limit,
        }),
    })
}

pub fn plan_universal_edit_v2(
    input: &[u8],
    request: &UniversalEditRequestV2,
) -> Result<UniversalEditPlanV2> {
    crate::cancel::check_current_cancel("universal edit planning snapshot")?;
    let snapshot = build_document_snapshot(input, None)?;
    let policy_engine = ContentEngine::open_bytes(input.to_vec())?;
    let secure_policy = analyze_edit_policy(&policy_engine, SignatureEditOperation::ContentEdit)?;
    let requested_operation = request.operation.clone();
    let mut execution_operation = requested_operation.clone();
    let mut state = UniversalPlanStateV2::Ready;
    let mut candidates = Vec::new();
    let mut selected_candidate_ids = Vec::new();
    let mut approval_reasons = Vec::new();
    let mut read_set = Vec::new();
    let mut write_set = Vec::new();
    let mut preview = json!({"kind": "source_and_geometry_diff", "status": "ready"});
    let mut signature_impact = json!({
        "mode": request.policy.mutation_mode,
        "cryptographic_validity_claimed": false,
    });
    let mut conformance_impact = json!({"requires_revalidation": true});
    let mut implementation_report;

    crate::cancel::check_current_cancel("universal edit operation planning")?;
    match &requested_operation {
        UniversalEditOperationV2::Text { request: text } => {
            let mut execution = text.clone();
            execution.signature_policy_override = request.policy.mutation_mode
                == UniversalMutationModeV2::AuthorizedRewrite;
            let mut planning_request = execution.clone();
            // Candidate validity is resolved below against the full logical
            // inventory. Route analysis must not throw early merely because a
            // caller supplied a stale or multi-run candidate identifier.
            planning_request.source_instruction_id = None;
            match plan_scene_text_transaction(input, &planning_request) {
                Ok(report) => {
                    read_set = report.read_set.clone();
                    write_set = report.write_set.clone();
                    signature_impact = report.signature_impact.clone();
                    conformance_impact = report.conformance_impact.clone();
                    if let Ok(provenance) = crate::source_editing::operator_text_provenance(
                        input,
                        text.page,
                        &text.source_text,
                        &text.replacement_text,
                    ) {
                        for identity in provenance.source_instructions.into_iter().filter(|identity| {
                            text.source_instruction_id
                                .as_deref()
                                .is_none_or(|selected| identity.instruction_id == selected)
                        }) {
                            let id = identity.instruction_id.clone();
                            candidates.push(UniversalCandidateV2 {
                                candidate_id: id.clone(),
                                page: text.page,
                                kind: "text_source_instruction".to_string(),
                                source_identity: serde_json::to_value(identity).map_err(json_error)?,
                                confidence: if report.refusal.is_none() { 1.0 } else { 0.70 },
                                exact: report.refusal.is_none(),
                                shared_resource: false,
                                approval_reason: report.refusal.as_ref().map(|_| "source route requires escalation or explicit candidate approval".to_string()),
                            });
                            selected_candidate_ids.push(id);
                        }
                    }
                    if report.refusal.is_some() {
                        promote_plan_state(&mut state, UniversalPlanStateV2::ApprovalRequired);
                        approval_reasons.push("operator-preserving route cannot satisfy the requested font/layout constraints; approve semantic reconstruction".to_string());
                        execution.requested_mode = if execution.region.is_some() {
                            crate::source_editing::TrueEditingMode::GeometricBlock
                        } else {
                            crate::source_editing::TrueEditingMode::SemanticDocument
                        };
                        execution.approve_low_confidence_structure = true;
                        if request.policy.allow_font_substitution {
                            execution.font_policy = "allow_substitute".to_string();
                        }
                    }
                    implementation_report = serde_json::to_value(report).map_err(json_error)?;
                }
                Err(error) if matches!(error, WellfriendError::UnsupportedFeature(_)) => {
                    promote_plan_state(&mut state, UniversalPlanStateV2::ApprovalRequired);
                    approval_reasons.push(error.to_string());
                    execution.requested_mode = if execution.region.is_some() {
                        crate::source_editing::TrueEditingMode::GeometricBlock
                    } else {
                        crate::source_editing::TrueEditingMode::SemanticDocument
                    };
                    execution.approve_low_confidence_structure = true;
                    if request.policy.allow_font_substitution {
                        execution.font_policy = "allow_substitute".to_string();
                    }
                    implementation_report = json!({"route": "semantic_reconstruction", "source_error": error.to_string()});
                }
                Err(error) => return Err(error),
            }
            let logical_candidates = logical_text_range_candidates_v2(
                input,
                text,
                &snapshot.revision_id,
                &candidates,
            )?;
            for candidate in logical_candidates {
                if candidate.exact {
                    selected_candidate_ids.push(candidate.candidate_id.clone());
                }
                candidates.push(candidate);
            }
            if candidates.is_empty() {
                promote_plan_state(&mut state, UniversalPlanStateV2::TargetNotFound);
                approval_reasons.push(
                    "the requested source text is not present in the selected page's provenance-bearing logical text"
                        .to_string(),
                );
            } else if !candidates.iter().any(|candidate| candidate.exact) {
                promote_plan_state(&mut state, UniversalPlanStateV2::PolicyDenied);
                approval_reasons.push(
                    "the visible text match contains a logical interval with no writable source provenance"
                        .to_string(),
                );
            }
            if candidates.len() > 1 {
                match request.policy.ambiguity {
                    UniversalAmbiguityPolicyV2::PreviewAndConfirm => {
                        promote_plan_state(&mut state, UniversalPlanStateV2::ApprovalRequired);
                        approval_reasons.push(
                            "multiple source instructions match the visual selection".to_string(),
                        );
                    }
                    UniversalAmbiguityPolicyV2::AutomaticExactOnly => {
                        promote_plan_state(&mut state, UniversalPlanStateV2::PolicyDenied);
                        approval_reasons.push(
                            "automatic_exact_only forbids choosing among multiple exact source instructions; provide source_instruction_id or replan with preview_and_confirm"
                                .to_string(),
                        );
                    }
                }
            }
            if let Some(selected) = text.source_instruction_id.as_deref().and_then(|id| {
                candidates
                    .iter()
                    .find(|candidate| candidate.candidate_id == id && candidate.exact)
            }) {
                bind_text_candidate_to_request(&mut execution, selected)?;
            }
            if execution.font_policy == "allow_substitute" {
                promote_plan_state(&mut state, UniversalPlanStateV2::ApprovalRequired);
                approval_reasons.push("font substitution may change metrics or pagination".to_string());
                let mut by_candidate = serde_json::Map::new();
                let mut any_approved_candidate = false;
                for candidate in candidates.iter().filter(|candidate| candidate.exact) {
                    let mut candidate_request = execution.clone();
                    bind_text_candidate_to_request(&mut candidate_request, candidate)?;
                    let requested_family = source_font_family(input, &candidate_request)
                        .unwrap_or_else(|| "unresolved_source_font".to_string());
                    let source_font_bytes = source_font_program(input, &candidate_request);
                    let report = universal_substitution_report_v2(
                        &requested_family,
                        &execution.replacement_text,
                        Some("allow_substitute"),
                        source_font_bytes.as_deref(),
                        execution.approved_font_asset.as_ref(),
                    );
                    any_approved_candidate |= report["approved_candidates"]
                        .as_array()
                        .is_some_and(|items| !items.is_empty());
                    by_candidate.insert(candidate.candidate_id.clone(), report);
                }
                if !any_approved_candidate {
                    promote_plan_state(&mut state, UniversalPlanStateV2::PolicyDenied);
                    approval_reasons.push(
                        "no governed substitute font has complete Unicode coverage for the replacement text"
                            .to_string(),
                    );
                }
                implementation_report = json!({
                    "transaction": implementation_report,
                    "font_substitution": {
                        "candidate_specific": true,
                        "by_candidate": by_candidate,
                    },
                    "approval_contract": "approved_font must be one exact ranked lookup_name for the selected candidate; apply binds that font's bytes to shaping, measurement, Type0 generation, and continuation pages",
                });
            }
            execution_operation = UniversalEditOperationV2::Text { request: execution };
            preview = json!({
                "page": text.page,
                "source_text": text.source_text,
                "replacement_text": text.replacement_text,
                "candidate_count": candidates.len(),
                "required_layers": ["source_highlight", "replacement_layout", "affected_object_diff", "signature_impact"],
            });
        }
        UniversalEditOperationV2::Image { request: image } => {
            validate_image_replacement(&image.replacement)?;
            let occurrences = universal_image_occurrences_v2(input, &[image.page])?;
            for occurrence in occurrences.into_iter().filter(|candidate| {
                candidate.page == image.page
                    &&
                image
                    .occurrence_id
                    .as_deref()
                    .is_none_or(|id| candidate.occurrence_id == id)
                    &&
                image
                    .object_number
                    .is_none_or(|number| {
                        candidate.object_number == Some(number)
                            && candidate.generation == Some(image.generation)
                    })
                    && image
                        .resource_name
                        .as_deref()
                        .is_none_or(|name| candidate.resource_name.as_deref() == Some(name))
            }) {
                let id = occurrence.occurrence_id.clone();
                let shared = occurrence.shared_definition_uses > 1;
                candidates.push(UniversalCandidateV2 {
                    candidate_id: id.clone(),
                    page: image.page,
                    kind: if occurrence.inline { "inline_image" } else { "image_xobject" }.to_string(),
                    source_identity: serde_json::to_value(&occurrence).map_err(json_error)?,
                    confidence: 1.0,
                    exact: true,
                    shared_resource: shared,
                    approval_reason: (occurrence.inline || shared).then(|| {
                        if occurrence.inline {
                            "inline image will be promoted to an occurrence-owned XObject".to_string()
                        } else {
                            "shared image definition requires explicit clone-one or edit-all policy".to_string()
                        }
                    }),
                });
            }
            if candidates.is_empty() {
                promote_plan_state(&mut state, UniversalPlanStateV2::TargetNotFound);
                approval_reasons.push("the requested image occurrence does not exist in the selected page resource graph".to_string());
            } else if candidates.len() > 1 {
                match request.policy.ambiguity {
                    UniversalAmbiguityPolicyV2::PreviewAndConfirm => {
                        promote_plan_state(&mut state, UniversalPlanStateV2::ApprovalRequired);
                        approval_reasons.push(
                            "select the exact image occurrence and clone-one or edit-all behavior"
                                .to_string(),
                        );
                    }
                    UniversalAmbiguityPolicyV2::AutomaticExactOnly => {
                        promote_plan_state(&mut state, UniversalPlanStateV2::PolicyDenied);
                        approval_reasons.push(
                            "automatic_exact_only forbids choosing among multiple image occurrences; provide occurrence_id or replan with preview_and_confirm"
                                .to_string(),
                        );
                    }
                }
            } else if image.occurrence_id.is_none()
                && request.policy.ambiguity == UniversalAmbiguityPolicyV2::PreviewAndConfirm
            {
                promote_plan_state(&mut state, UniversalPlanStateV2::ApprovalRequired);
                approval_reasons.push(
                    "confirm the unique image occurrence and clone-one or edit-all behavior"
                        .to_string(),
                );
            }
            selected_candidate_ids = image.occurrence_id.clone().into_iter().collect::<Vec<_>>();
            if selected_candidate_ids.is_empty()
                && candidates.len() == 1
                && request.policy.ambiguity == UniversalAmbiguityPolicyV2::AutomaticExactOnly
            {
                selected_candidate_ids.push(candidates[0].candidate_id.clone());
            }
            read_set = candidates
                .iter()
                .map(|candidate| candidate.candidate_id.clone())
                .collect();
            write_set = candidates
                .iter()
                .filter(|candidate| {
                    selected_candidate_ids.is_empty()
                        || selected_candidate_ids.contains(&candidate.candidate_id)
                })
                .filter_map(|candidate| {
                    let number = candidate.source_identity["object_number"]
                        .as_u64()
                        .or_else(|| candidate.source_identity["owner_stream_object"].as_u64())?;
                    let generation = candidate.source_identity["generation"]
                        .as_u64()
                        .or_else(|| {
                            candidate.source_identity["owner_stream_generation"].as_u64()
                        })
                        .unwrap_or(0);
                    Some(format!("object-{number}-{generation}"))
                })
                .collect();
            implementation_report = json!({
                "route": "source_owned_image_occurrence_mutation",
                "shared_resource_policy": image.shared_resource_policy,
                "nested_forms": "recursive_clone_on_write",
                "inline_images": "promote_selected_occurrence_to_xobject",
                "mask_policy": if image.replacement.image_mask { "replace_with_explicit_stencil_image_mask" } else if image.replacement.soft_mask.is_some() { "replace_with_explicit_devicegray_soft_mask" } else if image.replacement.preserve_masks { "preserve_under_typed_sample_model_validation" } else { "remove" },
            });
            preview = json!({
                "page": image.page,
                "candidate_count": candidates.len(),
                "replacement": {
                    "width": image.replacement.width,
                    "height": image.replacement.height,
                    "color_space": image.replacement.color_space,
                    "color_space_descriptor": image.replacement.color_space_descriptor,
                    "encoding": image.replacement.encoding,
                    "image_mask": image.replacement.image_mask,
                    "decode": image.replacement.decode,
                    "soft_mask": image.replacement.soft_mask.as_ref().map(|mask| json!({
                        "width": mask.width,
                        "height": mask.height,
                        "bits_per_component": mask.bits_per_component,
                        "samples_sha256": digest_hex(&mask.samples),
                    })),
                },
                "required_layers": ["selected_occurrence", "shared_uses", "mask_profile_diff", "replacement_preview"],
            });
        }
        UniversalEditOperationV2::Vector { request: vector } => {
            let inventory = list_vector_objects(input, vector.page)?;
            if let Some(object) = inventory
                .objects
                .iter()
                .find(|object| object.stable_id == vector.stable_id)
            {
                let shared = object.provenance.form_invocation.is_some();
                let id = stable_id(
                    "candidate",
                    &[snapshot.revision_id.as_bytes(), vector.stable_id.as_bytes()],
                );
                candidates.push(UniversalCandidateV2 {
                    candidate_id: id.clone(),
                    page: vector.page,
                    kind: "vector_source_range".to_string(),
                    source_identity: serde_json::to_value(object).map_err(json_error)?,
                    confidence: 1.0,
                    exact: true,
                    shared_resource: shared,
                    approval_reason: shared.then(|| "shared Form occurrence requires clone-one or edit-all approval".to_string()),
                });
                selected_candidate_ids.push(id);
                read_set.push(vector.stable_id.clone());
                write_set.push(format!(
                    "object-{}-{}",
                    object.provenance.object_number, object.provenance.generation
                ));
                if shared {
                    promote_plan_state(&mut state, UniversalPlanStateV2::ApprovalRequired);
                    approval_reasons.push("shared Form resource policy must be acknowledged".to_string());
                }
            } else {
                promote_plan_state(&mut state, UniversalPlanStateV2::TargetNotFound);
                approval_reasons.push("the selected vector identity is not present in the current revision".to_string());
            }
            implementation_report = json!({"route": "advanced_editing_vector_source_range"});
            preview = json!({"page": vector.page, "stable_id": vector.stable_id, "operation": vector.operation});
        }
        UniversalEditOperationV2::StructureCorrection { request: correction } => {
            let correction_json = serde_json::to_string(&json!({
                "semantic_node_id": correction.semantic_node_id,
                "accepted_relationships": correction.accepted_relationships,
            }))
            .map_err(json_error)?;
            let structure_approval =
                crate::text_reflow::approve_structure_correction(input, &correction_json)?;
            if structure_approval["node_page"].as_u64()
                != u64::try_from(correction.text_edit.page).ok()
            {
                return Err(WellfriendError::invalid_input(
                    "structure correction semantic node and text edit must belong to the same page",
                ));
            }
            promote_plan_state(&mut state, UniversalPlanStateV2::ApprovalRequired);
            approval_reasons.push("semantic structure corrections require explicit relationship approval".to_string());
            let mut execution = correction.clone();
            execution.text_edit.requested_mode =
                crate::source_editing::TrueEditingMode::SemanticDocument;
            execution.text_edit.approve_low_confidence_structure = true;
            execution.text_edit.signature_policy_override = request.policy.mutation_mode
                == UniversalMutationModeV2::AuthorizedRewrite;
            let mut semantic_planning_request = execution.text_edit.clone();
            semantic_planning_request.source_instruction_id = None;
            let semantic_plan = match plan_scene_text_transaction(input, &semantic_planning_request) {
                Ok(report) => {
                    read_set = report.read_set.clone();
                    write_set = report.write_set.clone();
                    signature_impact = report.signature_impact.clone();
                    conformance_impact = report.conformance_impact.clone();
                    serde_json::to_value(report).map_err(json_error)?
                }
                Err(error) if matches!(error, WellfriendError::UnsupportedFeature(_)) => {
                    approval_reasons.push(error.to_string());
                    json!({"status": "additional_semantic_reconstruction_required", "reason": error.to_string()})
                }
                Err(error) => return Err(error),
            };
            read_set.push(correction.semantic_node_id.clone());
            if let Ok(provenance) = crate::source_editing::operator_text_provenance(
                input,
                correction.text_edit.page,
                &correction.text_edit.source_text,
                &correction.text_edit.replacement_text,
            ) {
                for identity in provenance.source_instructions.into_iter().filter(|identity| {
                    correction
                        .text_edit
                        .source_instruction_id
                        .as_deref()
                        .is_none_or(|selected| identity.instruction_id == selected)
                }) {
                    let id = identity.instruction_id.clone();
                    candidates.push(UniversalCandidateV2 {
                        candidate_id: id.clone(),
                        page: correction.text_edit.page,
                        kind: "text_source_instruction".to_string(),
                        source_identity: serde_json::to_value(identity).map_err(json_error)?,
                        confidence: 0.90,
                        exact: true,
                        shared_resource: false,
                        approval_reason: Some(
                            "structure correction binds the semantic decision to this exact source instruction"
                                .to_string(),
                        ),
                    });
                    selected_candidate_ids.push(id);
                }
            }
            let logical_candidates = logical_text_range_candidates_v2(
                input,
                &correction.text_edit,
                &snapshot.revision_id,
                &candidates,
            )?;
            for candidate in logical_candidates {
                if candidate.exact {
                    selected_candidate_ids.push(candidate.candidate_id.clone());
                }
                candidates.push(candidate);
            }
            if candidates.is_empty() {
                promote_plan_state(&mut state, UniversalPlanStateV2::TargetNotFound);
                approval_reasons.push(
                    "the structure correction's source text is absent from the selected page"
                        .to_string(),
                );
            } else if !candidates.iter().any(|candidate| candidate.exact) {
                promote_plan_state(&mut state, UniversalPlanStateV2::PolicyDenied);
                approval_reasons.push(
                    "the structure correction's text contains a logical interval with no writable source provenance"
                        .to_string(),
                );
            }
            if candidates.len() > 1 {
                match request.policy.ambiguity {
                    UniversalAmbiguityPolicyV2::PreviewAndConfirm => approval_reasons.push(
                        "select the exact source instruction for the structure correction"
                            .to_string(),
                    ),
                    UniversalAmbiguityPolicyV2::AutomaticExactOnly => {
                        promote_plan_state(&mut state, UniversalPlanStateV2::PolicyDenied);
                        approval_reasons.push(
                            "automatic_exact_only forbids choosing among multiple structure-correction source instructions; provide source_instruction_id or replan with preview_and_confirm"
                                .to_string(),
                        );
                    }
                }
            }
            if let Some(selected) = correction
                .text_edit
                .source_instruction_id
                .as_deref()
                .and_then(|id| {
                    candidates
                        .iter()
                        .find(|candidate| candidate.candidate_id == id && candidate.exact)
                })
            {
                bind_text_candidate_to_request(&mut execution.text_edit, selected)?;
            }
            let font_substitution = if execution.text_edit.font_policy == "allow_substitute" {
                let mut by_candidate = serde_json::Map::new();
                let mut any_approved_candidate = false;
                for candidate in candidates.iter().filter(|candidate| candidate.exact) {
                    let mut candidate_request = execution.text_edit.clone();
                    bind_text_candidate_to_request(&mut candidate_request, candidate)?;
                    let requested_family = source_font_family(input, &candidate_request)
                        .unwrap_or_else(|| "unresolved_source_font".to_string());
                    let source_font_bytes = source_font_program(input, &candidate_request);
                    let report = universal_substitution_report_v2(
                        &requested_family,
                        &candidate_request.replacement_text,
                        Some("allow_substitute"),
                        source_font_bytes.as_deref(),
                        candidate_request.approved_font_asset.as_ref(),
                    );
                    any_approved_candidate |= report["approved_candidates"]
                        .as_array()
                        .is_some_and(|items| !items.is_empty());
                    by_candidate.insert(candidate.candidate_id.clone(), report);
                }
                if !any_approved_candidate {
                    promote_plan_state(&mut state, UniversalPlanStateV2::PolicyDenied);
                    approval_reasons.push(
                        "no governed substitute font has complete Unicode coverage for the structure correction"
                            .to_string(),
                    );
                }
                Some(json!({
                    "candidate_specific": true,
                    "by_candidate": by_candidate,
                }))
            } else {
                None
            };
            execution_operation = UniversalEditOperationV2::StructureCorrection {
                request: execution,
            };
            if correction.repair_tagged_structure {
                write_set.push("catalog.StructTreeRoot.ParentTree".to_string());
                write_set.push("pages.StructParents".to_string());
            }
            implementation_report = json!({
                "route": "approval_guided_semantic_document_source_transaction",
                "structure_approval": structure_approval,
                "semantic_text_plan": semantic_plan,
                "font_substitution": font_substitution,
                "tag_tree_mutation_claimed": correction.repair_tagged_structure,
                "structure_repair_route": if correction.repair_tagged_structure { "DocumentSecurity::RepairAfterMutation(TextEdit)" } else { "caller_owned_object_graph" },
            });
            preview = json!({
                "page": correction.text_edit.page,
                "semantic_node_id": correction.semantic_node_id,
                "required_layers": ["semantic_node", "accepted_relationships", "text_reflow_diff", "tag_tree_parent_tree_diff"],
            });
        }
        UniversalEditOperationV2::DocumentSubsystem { request: subsystem } => {
            let mut execution = subsystem.clone();
            execution.approved = true;
            let subsystem_plan = crate::document_subsystems::plan_document_subsystems(
                input,
                &execution,
            )?;
            let candidate_id = stable_id(
                "document-subsystem-v2",
                &[
                    snapshot.revision_id.as_bytes(),
                    serde_json::to_vec(subsystem).map_err(json_error)?.as_slice(),
                ],
            );
            candidates.push(UniversalCandidateV2 {
                candidate_id: candidate_id.clone(),
                page: subsystem
                    .reflow
                    .as_ref()
                    .map(|reflow| reflow.page)
                    .or_else(|| document_subsystem_action_page_v2(subsystem.action.as_ref()))
                    .unwrap_or(1),
                kind: "document_subsystem_transaction".to_string(),
                source_identity: json!({
                    "revision_id": snapshot.revision_id,
                    "subsystem": subsystem.subsystem,
                    "action": subsystem.action,
                }),
                confidence: 1.0,
                exact: true,
                shared_resource: false,
                approval_reason: Some(
                    "typed document-subsystem mutation requires revision-bound approval"
                        .to_string(),
                ),
            });
            selected_candidate_ids.push(candidate_id);
            promote_plan_state(&mut state, UniversalPlanStateV2::ApprovalRequired);
            approval_reasons.push(
                "approve the typed OCR/table/math/form/annotation/XFA source transaction"
                    .to_string(),
            );
            read_set.push("document_subsystems.analysis".to_string());
            write_set.push("document_subsystems.typed_source_graph".to_string());
            execution_operation = UniversalEditOperationV2::DocumentSubsystem {
                request: execution,
            };
            implementation_report = json!({
                "route": "document_subsystems_typed_transaction",
                "plan": subsystem_plan,
                "ocr_provider_results_supported": true,
            });
            preview = json!({
                "kind": "document_subsystem_source_diff",
                "subsystem": subsystem.subsystem,
                "action": subsystem.action,
            });
        }
        UniversalEditOperationV2::DocumentSecurity { request: security } => {
            let mut execution = security.clone();
            execution.approved = true;
            if request.policy.mutation_mode == UniversalMutationModeV2::AuthorizedRewrite {
                execution.full_rewrite_acknowledged = true;
            }
            let security_plan = crate::document_security::plan_document_security(
                input,
                &execution,
            )?;
            let candidate_id = stable_id(
                "document-security-v2",
                &[
                    snapshot.revision_id.as_bytes(),
                    serde_json::to_vec(security).map_err(json_error)?.as_slice(),
                ],
            );
            candidates.push(UniversalCandidateV2 {
                candidate_id: candidate_id.clone(),
                page: security_plan.changed_pages.first().copied().unwrap_or(1),
                kind: "document_security_transaction".to_string(),
                source_identity: json!({
                    "revision_id": snapshot.revision_id,
                    "subsystem": security.subsystem,
                    "action": security.action,
                }),
                confidence: 1.0,
                exact: true,
                shared_resource: true,
                approval_reason: Some(
                    "tag-tree, ParentTree, redaction, or sanitization mutation requires explicit structural approval"
                        .to_string(),
                ),
            });
            selected_candidate_ids.push(candidate_id);
            promote_plan_state(&mut state, UniversalPlanStateV2::ApprovalRequired);
            approval_reasons.push(
                "approve the typed document-security and semantic-structure transaction"
                    .to_string(),
            );
            read_set.extend(security_plan.read_set.clone());
            write_set.extend(security_plan.write_set.clone());
            execution_operation = UniversalEditOperationV2::DocumentSecurity {
                request: execution,
            };
            implementation_report = json!({
                "route": "document_security_typed_transaction",
                "plan": security_plan,
                "tag_tree_mutation_claimed": true,
            });
            preview = json!({
                "kind": "semantic_structure_and_security_diff",
                "subsystem": security.subsystem,
                "action": security.action,
            });
        }
        UniversalEditOperationV2::ObjectGraph { request: object_graph } => {
            let planned = plan_object_graph_edit_v2(input, object_graph, &snapshot.revision_id)?;
            candidates = planned.candidates;
            selected_candidate_ids = candidates
                .iter()
                .map(|candidate| candidate.candidate_id.clone())
                .collect();
            read_set = planned.read_set;
            write_set = planned.write_set;
            promote_plan_state(&mut state, UniversalPlanStateV2::ApprovalRequired);
            approval_reasons.push(
                "approve exact indirect-object graph replacement and its declared render-invalidation scope"
                    .to_string(),
            );
            implementation_report = planned.report;
            preview = json!({
                "kind": "indirect_object_graph_diff",
                "mutation_count": object_graph.mutations.len(),
                "affected_pages": object_graph.affected_pages,
                "global_resource_impact": object_graph.acknowledge_global_resource_impact,
            });
        }
    }

    approval_reasons.sort();
    approval_reasons.dedup();
    read_set.sort();
    read_set.dedup();
    write_set.sort();
    write_set.dedup();
    let route_signature_impact = signature_impact;
    signature_impact = json!({
        "mode": request.policy.mutation_mode,
        "secure_mutation_policy": &secure_policy,
        "route_specific_impact": route_signature_impact,
        "cryptographic_validity_claimed": false,
    });
    match request.policy.mutation_mode {
        UniversalMutationModeV2::PreserveSignatures
            if matches!(
                secure_policy.decision,
                EditPolicyDecision::BlockedBySignaturePolicy
                    | EditPolicyDecision::ExplicitOverrideRequired
                    | EditPolicyDecision::FullRewriteRequired
            ) =>
        {
            promote_plan_state(&mut state, UniversalPlanStateV2::PolicyDenied);
            approval_reasons.push(
                "preserve_signatures mode forbids this mutation under the current DocMDP/FieldMDP/signature policy"
                    .to_string(),
            );
        }
        UniversalMutationModeV2::AuthorizedRewrite
            if !secure_policy.structural_policies.is_empty()
                || !secure_policy.cryptographic_reports.is_empty() =>
        {
            promote_plan_state(&mut state, UniversalPlanStateV2::ApprovalRequired);
            approval_reasons.push(
                "authorized_rewrite requires explicit acknowledgement that earlier signatures may be invalidated"
                    .to_string(),
            );
        }
        _ => {}
    }
    let input_recovery = universal_input_recovery_state(input);
    if input_recovery["strict_open"] == Value::Bool(false) {
        match (
            request.policy.allow_deterministic_repair,
            request.policy.mutation_mode,
        ) {
            (false, _) => {
                promote_plan_state(&mut state, UniversalPlanStateV2::PolicyDenied);
                approval_reasons.push(
                    "input requires deterministic normalization but allow_deterministic_repair is false"
                        .to_string(),
                );
            }
            (true, UniversalMutationModeV2::PreserveSignatures) => {
                promote_plan_state(&mut state, UniversalPlanStateV2::PolicyDenied);
                approval_reasons.push(
                    "deterministic input normalization is a full rewrite; replan in authorized_rewrite mode"
                        .to_string(),
                );
            }
            (true, UniversalMutationModeV2::AuthorizedRewrite) => {
                promote_plan_state(&mut state, UniversalPlanStateV2::ApprovalRequired);
                approval_reasons.push(
                    "approve deterministic full-rewrite normalization after the source mutation"
                        .to_string(),
                );
            }
        }
    }
    let required_conformance_profiles =
        required_conformance_profiles_v2(input, &request.policy)?;
    if request.policy.require_conformance_preservation
        || !request.policy.required_conformance_profiles.is_empty()
    {
        promote_plan_state(&mut state, UniversalPlanStateV2::ApprovalRequired);
        approval_reasons.push(
            "approve fail-closed post-mutation validation of every required PDF/A, PDF/UA, and PDF/X profile"
                .to_string(),
        );
        let route_specific_impact = conformance_impact;
        conformance_impact = json!({
            "requested_guarantee": "preserve_declared_conformance",
            "guarantee_proven": false,
            "decision": "validate_final_bytes_before_return",
            "required_profiles": required_conformance_profiles.iter().map(|profile| profile.label()).collect::<Vec<_>>(),
            "empty_profile_set_means_no_source_declaration_detected": required_conformance_profiles.is_empty(),
            "external_certification_claimed": false,
            "route_specific_impact": route_specific_impact,
        });
    }
    if let UniversalOutputSecurityPolicyV2::Standard {
        algorithm,
        permissions,
        encrypt_metadata,
    } = &request.policy.output_security
    {
        if matches!(
            *algorithm,
            UniversalStandardEncryptionAlgorithmV2::Rc4_128
                | UniversalStandardEncryptionAlgorithmV2::Aes128
        ) {
            promote_plan_state(&mut state, UniversalPlanStateV2::PolicyDenied);
            approval_reasons.push(
                "legacy RC4-128/AES-128 output is disabled in universal editing because the current writer lacks independent cross-reader interoperability proof"
                    .to_string(),
            );
        }
        if request.policy.mutation_mode != UniversalMutationModeV2::AuthorizedRewrite {
            promote_plan_state(&mut state, UniversalPlanStateV2::PolicyDenied);
            approval_reasons.push(
                "standard output re-encryption is a full rewrite; use authorized_rewrite mode"
                    .to_string(),
            );
        } else {
            promote_plan_state(&mut state, UniversalPlanStateV2::ApprovalRequired);
            approval_reasons.push(
                "approve credential-bound full-rewrite output re-encryption and security-envelope rotation"
                    .to_string(),
            );
        }
        if required_conformance_profiles.iter().any(|profile| {
            matches!(
                profile,
                UniversalConformanceProfileV2::PdfA1B
                    | UniversalConformanceProfileV2::PdfA2B
                    | UniversalConformanceProfileV2::PdfA2A
                    | UniversalConformanceProfileV2::PdfA3B
                    | UniversalConformanceProfileV2::PdfA3A
                    | UniversalConformanceProfileV2::PdfX1A2001
                    | UniversalConformanceProfileV2::PdfX3_2003
                    | UniversalConformanceProfileV2::PdfX4
            )
        }) {
            promote_plan_state(&mut state, UniversalPlanStateV2::PolicyDenied);
            approval_reasons.push(
                "PDF/A and PDF/X prohibit encryption; the requested output-security and conformance policies conflict"
                    .to_string(),
            );
        }
        implementation_report = json!({
            "operation": implementation_report,
            "output_security": {
                "kind": "standard",
                "algorithm": algorithm,
                "permissions": permissions,
                "encrypt_metadata": encrypt_metadata,
                "credentials_in_plan": false,
                "credential_transport": "apply_only_non_serializable",
                "legacy_rc4_aes128_policy": "fail_closed_pending_independent_interoperability_qualification",
            }
        });
    }
    implementation_report = json!({
        "operation": implementation_report,
        "input_recovery": input_recovery,
        "repair_commit_order": "source mutation in memory, deterministic normalization, strict reopen, then return output",
    });
    approval_reasons.sort();
    approval_reasons.dedup();
    let plan_id = plan_id(
        &snapshot.revision_id,
        &requested_operation,
        &execution_operation,
        &request.policy,
    )?;
    Ok(UniversalEditPlanV2 {
        schema_version: UNIVERSAL_EDITING_SCHEMA_VERSION.to_string(),
        plan_id,
        document_id: snapshot.document_id,
        revision_id: snapshot.revision_id,
        snapshot_id: snapshot.snapshot_id,
        state,
        requested_operation,
        execution_operation,
        policy: request.policy.clone(),
        candidates,
        selected_candidate_ids,
        approval_reasons,
        read_set,
        write_set,
        preview,
        signature_impact,
        conformance_impact,
        implementation_report,
    })
}

pub fn create_universal_approval_token_v2(
    plan: &UniversalEditPlanV2,
    decision: UniversalApprovalDecisionV2,
) -> Result<UniversalApprovalTokenV2> {
    if plan.state != UniversalPlanStateV2::ApprovalRequired {
        return Err(WellfriendError::invalid_input(
            "universal editing approval token is only valid for approval_required plans",
        ));
    }
    if decision.mutation_mode != plan.policy.mutation_mode {
        return Err(WellfriendError::invalid_input(
            "approval mutation mode differs from the planned mutation mode",
        ));
    }
    if decision.mutation_mode == UniversalMutationModeV2::AuthorizedRewrite
        && !decision.accept_signature_invalidation
    {
        return Err(WellfriendError::invalid_input(
            "authorized rewrite approval must acknowledge signature invalidation",
        ));
    }
    if !decision.accept_visual_change {
        return Err(WellfriendError::invalid_input(
            "universal editing approval must explicitly accept the planned visual or structural change",
        ));
    }
    let known = plan
        .candidates
        .iter()
        .map(|candidate| candidate.candidate_id.as_str())
        .collect::<BTreeSet<_>>();
    if decision
        .selected_candidate_ids
        .iter()
        .any(|candidate| !known.contains(candidate.as_str()))
    {
        return Err(WellfriendError::invalid_input(
            "approval selected a candidate outside the revision-bound plan",
        ));
    }
    if decision.selected_candidate_ids.iter().any(|selected| {
        plan.candidates
            .iter()
            .find(|candidate| candidate.candidate_id == *selected)
            .is_some_and(|candidate| !candidate.exact)
    }) {
        return Err(WellfriendError::invalid_input(
            "approval selected a non-exact candidate that cannot be applied",
        ));
    }
    if matches!(&plan.requested_operation, UniversalEditOperationV2::Image { .. })
        && decision.selected_candidate_ids.len() != 1
    {
        return Err(WellfriendError::invalid_input(
            "universal image approval must select exactly one occurrence candidate",
        ));
    }
    let text_candidates = plan
        .candidates
        .iter()
        .filter(|candidate| {
            matches!(
                candidate.kind.as_str(),
                "text_source_instruction" | "multi_run_text_range"
            )
        })
        .count();
    let selected_text_candidates = decision
        .selected_candidate_ids
        .iter()
        .filter(|selected| {
            plan.candidates
                .iter()
                .find(|candidate| candidate.candidate_id.as_str() == selected.as_str())
                .is_some_and(|candidate| {
                    matches!(
                        candidate.kind.as_str(),
                        "text_source_instruction" | "multi_run_text_range"
                    )
                })
        })
        .count();
    if selected_text_candidates > 1
        || (text_candidates > 1 && selected_text_candidates != 1)
    {
        return Err(WellfriendError::invalid_input(
            "universal approval must select exactly one source instruction when a text selection is ambiguous",
        ));
    }
    if matches!(&plan.requested_operation, UniversalEditOperationV2::ObjectGraph { .. })
        && decision
            .selected_candidate_ids
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>()
            != known
    {
        return Err(WellfriendError::invalid_input(
            "universal object-graph approval must select every revision-bound mutation candidate",
        ));
    }
    let font_policy = match &plan.execution_operation {
        UniversalEditOperationV2::Text { request } => Some(request.font_policy.as_str()),
        UniversalEditOperationV2::StructureCorrection { request } => {
            Some(request.text_edit.font_policy.as_str())
        }
        _ => None,
    };
    if let Some(font_policy) = font_policy {
        if font_policy == "allow_substitute" {
            let approved_font = decision.approved_font.as_deref().ok_or_else(|| {
                WellfriendError::invalid_input(
                    "universal text approval must select one ranked approved substitute font",
                )
            })?;
            let selected_candidate = decision
                .selected_candidate_ids
                .first()
                .map(String::as_str)
                .or_else(|| {
                    (plan.selected_candidate_ids.len() == 1)
                        .then(|| plan.selected_candidate_ids[0].as_str())
                })
                .ok_or_else(|| {
                    WellfriendError::invalid_input(
                        "font substitution approval requires one exact text candidate",
                    )
                })?;
            let pointer = format!(
                "/operation/font_substitution/by_candidate/{selected_candidate}/approved_candidates"
            );
            let ranked_fonts = plan
                .implementation_report
                .pointer(&pointer)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect::<BTreeSet<_>>();
            if !ranked_fonts.contains(approved_font) {
                return Err(WellfriendError::invalid_input(
                    "universal text approval selected a font outside the plan's ranked approved candidates",
                ));
            }
        } else if decision.approved_font.is_some() {
            return Err(WellfriendError::invalid_input(
                "universal approval supplied approved_font for a plan that does not substitute fonts",
            ));
        }
    }
    let binding_digest = approval_digest(plan, &decision)?;
    Ok(UniversalApprovalTokenV2 {
        schema_version: UNIVERSAL_EDITING_SCHEMA_VERSION.to_string(),
        plan_id: plan.plan_id.clone(),
        revision_id: plan.revision_id.clone(),
        decision,
        binding_digest,
    })
}

/// Inventory every image paint occurrence in the selected page window. The
/// walker follows nested Form XObjects, retains exact decoded-stream source
/// ranges, and treats inline image payloads as opaque binary data.
pub fn universal_image_occurrences_v2(
    input: &[u8],
    pages: &[usize],
) -> Result<Vec<UniversalImageOccurrenceV2>> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let reader = engine.document().reader();
    let revision = revision_id(input);
    let requested_pages = pages.iter().copied().collect::<BTreeSet<_>>();
    if requested_pages.is_empty() {
        return Ok(Vec::new());
    }
    let page_count = engine.page_count()?;
    if requested_pages
        .iter()
        .any(|page| *page == 0 || *page > page_count)
    {
        return Err(WellfriendError::invalid_input(
            "universal image occurrence page is outside the document",
        ));
    }
    let mut occurrences = Vec::new();
    let mut definition_counts = std::collections::BTreeMap::<(u32, u16), usize>::new();
    let mut global_occurrence_count = 0usize;
    // Definition sharing is a document property, not a page-window property.
    // Walk every page under one global occurrence budget, but retain detailed
    // candidates only for the requested page window.
    for page_number in 1..=page_count {
        crate::cancel::check_current_cancel("universal image occurrence page scan")?;
        let page = engine.document().get_page(page_number)?;
        let mut page_occurrences = Vec::new();
        let mut page_graphics_state = ImageOccurrenceGraphicsStateV2::new(
            UniversalImageMatrixV2::IDENTITY,
        );
        for (stream_index, (number, generation)) in page.contents.iter().copied().enumerate() {
            crate::cancel::check_current_cancel("universal image occurrence stream decode")?;
            let object = reader.get_object(number, generation)?;
            let decoded = decode_stream_lossless_with_limits(
                &object,
                reader,
                &DecodeLimits {
                    max_decoded_bytes_per_stream: 512 * 1024 * 1024,
                    ..DecodeLimits::default()
                },
            )?;
            if decoded.status != StreamDecodeStatus::Complete {
                return Err(WellfriendError::UnsupportedFeature(format!(
                    "universal image occurrence analysis cannot losslessly decode page {page_number} content stream {number} {generation} R"
                )));
            }
            collect_image_occurrences_in_stream(
                reader,
                &revision,
                page_number,
                stream_index,
                number,
                generation,
                &decoded.data,
                &page.resources,
                &mut page_graphics_state,
                &[],
                &mut Vec::new(),
                &mut page_occurrences,
            )?;
        }
        if !page_graphics_state.stack.is_empty() {
            return Err(WellfriendError::MalformedPdf(format!(
                "universal image analysis found unterminated graphics-state saves across page {page_number} /Contents"
            )));
        }
        global_occurrence_count = global_occurrence_count
            .checked_add(page_occurrences.len())
            .ok_or_else(|| {
                WellfriendError::ResourceLimit(
                    "universal image occurrence count overflow".to_string(),
                )
            })?;
        if global_occurrence_count > MAX_IMAGE_OCCURRENCES {
            return Err(WellfriendError::ResourceLimit(
                "universal image occurrence count exceeds the governed document limit 1000000"
                    .to_string(),
            ));
        }
        for occurrence in &page_occurrences {
            if let Some(key) = occurrence.object_number.zip(occurrence.generation) {
                *definition_counts.entry(key).or_insert(0) += 1;
            }
        }
        if requested_pages.contains(&page_number) {
            occurrences.append(&mut page_occurrences);
        }
    }
    for occurrence in &mut occurrences {
        occurrence.shared_definition_uses = occurrence
            .object_number
            .zip(occurrence.generation)
            .and_then(|key| definition_counts.get(&key).copied())
            .unwrap_or(1);
    }
    occurrences.sort_by_key(|occurrence| {
        (
            occurrence.page,
            occurrence.content_stream_index,
            occurrence.owner_stream_object,
            occurrence.operation_byte_start,
            occurrence.occurrence_id.clone(),
        )
    });
    Ok(occurrences)
}

#[derive(Debug)]
struct InlineImageBuilderV2 {
    start: usize,
    matrix: UniversalImageMatrixV2,
    parameters: Vec<SpannedContentToken>,
    data_len: usize,
}

#[derive(Debug, Clone)]
struct ImageOccurrenceGraphicsStateV2 {
    matrix: UniversalImageMatrixV2,
    stack: Vec<UniversalImageMatrixV2>,
}

impl ImageOccurrenceGraphicsStateV2 {
    fn new(matrix: UniversalImageMatrixV2) -> Self {
        Self {
            matrix,
            stack: Vec::new(),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_image_occurrences_in_stream(
    reader: &crate::reader::PdfReader,
    revision: &str,
    page: usize,
    stream_index: usize,
    owner_number: u32,
    owner_generation: u16,
    data: &[u8],
    resources: &PdfDictionary,
    graphics_state: &mut ImageOccurrenceGraphicsStateV2,
    invocation_path: &[VectorFormInvocation],
    active_forms: &mut Vec<(u32, u16)>,
    output: &mut Vec<UniversalImageOccurrenceV2>,
) -> Result<()> {
    if invocation_path.len() >= 64 {
        return Err(WellfriendError::UnsupportedFeature(
            "universal image Form recursion exceeds the governed depth limit 64".to_string(),
        ));
    }
    if output.len() >= MAX_IMAGE_OCCURRENCES {
        return Err(WellfriendError::ResourceLimit(
            "universal image occurrence count exceeds the governed limit 1000000".to_string(),
        ));
    }
    let xobjects = resolve_universal_dict(resources.get("XObject"), reader)
        .unwrap_or_else(PdfDictionary::empty);
    let mut tokenizer = ContentTokenizer::new(data);
    let mut operands = Vec::<SpannedContentToken>::new();
    let mut inline: Option<InlineImageBuilderV2> = None;
    let mut token_count = 0usize;
    while let Some(spanned) = tokenizer.next_spanned()? {
        token_count += 1;
        if token_count % 256 == 0 {
            crate::cancel::check_current_cancel(
                "universal image occurrence content-token scan",
            )?;
        }
        if let Some(builder) = inline.as_mut() {
            match &spanned.token {
                ContentToken::InlineImageData(bytes) => builder.data_len = bytes.len(),
                ContentToken::Operator(operator) if operator == "EI" => {
                    let builder = inline.take().ok_or_else(|| {
                        WellfriendError::MalformedPdf(
                            "universal image analysis lost inline-image parser state"
                                .to_string(),
                        )
                    })?;
                    let (width, height, bits, color_space, filters) =
                        inline_image_metadata(&builder.parameters);
                    let occurrence_id = image_occurrence_id(
                        revision,
                        page,
                        owner_number,
                        owner_generation,
                        builder.start,
                        spanned.end,
                        None,
                        invocation_path,
                    );
                    push_image_occurrence(output, UniversalImageOccurrenceV2 {
                        occurrence_id,
                        page,
                        content_stream_index: stream_index,
                        owner_stream_object: owner_number,
                        owner_stream_generation: owner_generation,
                        operation_byte_start: builder.start,
                        operation_byte_end: spanned.end,
                        resource_name: None,
                        object_number: None,
                        generation: None,
                        inline: true,
                        transform: builder.matrix,
                        bbox: image_bbox(builder.matrix),
                        width,
                        height,
                        bits_per_component: bits,
                        color_space,
                        filters,
                        invocation_path: invocation_path.to_vec(),
                        shared_definition_uses: 1,
                        clone_one_eligible: builder.data_len > 0,
                        edit_all_eligible: false,
                    })?;
                }
                _ => builder.parameters.push(spanned),
            }
            continue;
        }
        match &spanned.token {
            ContentToken::Operator(operator) => {
                let operation_start = operands
                    .first()
                    .map(|operand| operand.start)
                    .unwrap_or(spanned.start);
                match operator.as_str() {
                    "BI" => {
                        inline = Some(InlineImageBuilderV2 {
                            start: spanned.start,
                            matrix: graphics_state.matrix,
                            parameters: Vec::new(),
                            data_len: 0,
                        });
                    }
                    "q" => {
                        if !operands.is_empty() {
                            return Err(WellfriendError::MalformedPdf(
                                "universal image analysis found operands before q".to_string(),
                            ));
                        }
                        const MAX_IMAGE_GRAPHICS_STATE_DEPTH: usize = 4096;
                        if graphics_state.stack.len() >= MAX_IMAGE_GRAPHICS_STATE_DEPTH {
                            return Err(WellfriendError::ResourceLimit(format!(
                                "universal image graphics-state depth exceeds {MAX_IMAGE_GRAPHICS_STATE_DEPTH}"
                            )));
                        }
                        graphics_state.stack.push(graphics_state.matrix);
                    }
                    "Q" => {
                        if !operands.is_empty() {
                            return Err(WellfriendError::MalformedPdf(
                                "universal image analysis found operands before Q".to_string(),
                            ));
                        }
                        graphics_state.matrix = graphics_state.stack.pop().ok_or_else(|| {
                            WellfriendError::MalformedPdf(
                                "universal image analysis found graphics-state restore underflow"
                                    .to_string(),
                            )
                        })?;
                    }
                    "cm" => {
                        let values = content_numbers(&operands);
                        if operands.len() != 6 || values.len() != 6 {
                            return Err(WellfriendError::MalformedPdf(
                                "universal image analysis requires exactly six numeric cm operands"
                                    .to_string(),
                            ));
                        }
                        graphics_state.matrix = graphics_state.matrix.multiply(UniversalImageMatrixV2 {
                            a: values[0],
                            b: values[1],
                            c: values[2],
                            d: values[3],
                            e: values[4],
                            f: values[5],
                        });
                    }
                    "Do" => {
                        let [operand] = operands.as_slice() else {
                            return Err(WellfriendError::MalformedPdf(
                                "universal image analysis requires exactly one Do operand"
                                    .to_string(),
                            ));
                        };
                        let ContentToken::Name(name) = &operand.token else {
                            return Err(WellfriendError::MalformedPdf(
                                "universal image analysis requires a name operand for Do"
                                    .to_string(),
                            ));
                        };
                        let name = name.clone();
                        let Some((number, generation)) =
                            xobjects.get(&name).and_then(PdfObject::as_reference)
                        else {
                            operands.clear();
                            continue;
                        };
                        let object = reader.get_object(number, generation)?;
                        let PdfObject::Stream { dict, .. } = &object else {
                            operands.clear();
                            continue;
                        };
                        match dict.get_name("Subtype") {
                            Some("Image") => {
                                let occurrence_id = image_occurrence_id(
                                    revision,
                                    page,
                                    owner_number,
                                    owner_generation,
                                    operation_start,
                                    spanned.end,
                                    Some(&name),
                                    invocation_path,
                                );
                                push_image_occurrence(output, UniversalImageOccurrenceV2 {
                                    occurrence_id,
                                    page,
                                    content_stream_index: stream_index,
                                    owner_stream_object: owner_number,
                                    owner_stream_generation: owner_generation,
                                    operation_byte_start: operation_start,
                                    operation_byte_end: spanned.end,
                                    resource_name: Some(name),
                                    object_number: Some(number),
                                    generation: Some(generation),
                                    inline: false,
                                    transform: graphics_state.matrix,
                                    bbox: image_bbox(graphics_state.matrix),
                                    width: pdf_u32(dict.get_integer("Width").or_else(|| dict.get_integer("W"))),
                                    height: pdf_u32(dict.get_integer("Height").or_else(|| dict.get_integer("H"))),
                                    bits_per_component: pdf_u8(dict.get_integer("BitsPerComponent").or_else(|| dict.get_integer("BPC"))),
                                    color_space: image_color_space(dict),
                                    filters: image_filter_names(dict),
                                    invocation_path: invocation_path.to_vec(),
                                    shared_definition_uses: 1,
                                    clone_one_eligible: true,
                                    edit_all_eligible: true,
                                })?;
                            }
                            Some("Form") => {
                                if active_forms.contains(&(number, generation)) {
                                    return Err(WellfriendError::MalformedPdf(format!(
                                        "universal image analysis found cyclic Form XObject {number} {generation} R"
                                    )));
                                }
                                let decoded = decode_stream_lossless_with_limits(
                                    &object,
                                    reader,
                                    &DecodeLimits {
                                        max_decoded_bytes_per_stream: 512 * 1024 * 1024,
                                        ..DecodeLimits::default()
                                    },
                                )?;
                                if decoded.status != StreamDecodeStatus::Complete {
                                    return Err(WellfriendError::UnsupportedFeature(format!(
                                        "universal image analysis cannot losslessly decode Form {number} {generation} R"
                                    )));
                                }
                                let form_matrix = image_matrix_from_object(dict.get("Matrix"))
                                    .unwrap_or(UniversalImageMatrixV2::IDENTITY);
                                let nested_resources = resolve_universal_dict(
                                    dict.get("Resources"),
                                    reader,
                                )
                                .unwrap_or_else(|| resources.clone());
                                let mut nested_path = invocation_path.to_vec();
                                nested_path.push(VectorFormInvocation {
                                    resource_name: name,
                                    owner_stream_object: owner_number,
                                    owner_stream_generation: owner_generation,
                                    owner_operation_byte_start: operation_start,
                                    owner_operation_byte_end: spanned.end,
                                    form_object: number,
                                    form_generation: generation,
                                    depth: nested_path.len() + 1,
                                });
                                active_forms.push((number, generation));
                                let mut nested_graphics_state = ImageOccurrenceGraphicsStateV2::new(
                                    graphics_state.matrix.multiply(form_matrix),
                                );
                                collect_image_occurrences_in_stream(
                                    reader,
                                    revision,
                                    page,
                                    stream_index,
                                    number,
                                    generation,
                                    &decoded.data,
                                    &nested_resources,
                                    &mut nested_graphics_state,
                                    &nested_path,
                                    active_forms,
                                    output,
                                )?;
                                if !nested_graphics_state.stack.is_empty() {
                                    return Err(WellfriendError::MalformedPdf(format!(
                                        "universal image analysis found unterminated graphics-state saves in Form {number} {generation} R"
                                    )));
                                }
                                active_forms.pop();
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
                operands.clear();
            }
            _ => operands.push(spanned),
        }
    }
    if inline.is_some() {
        return Err(WellfriendError::MalformedPdf(
            "universal image analysis found an unterminated inline image".to_string(),
        ));
    }
    Ok(())
}

fn push_image_occurrence(
    output: &mut Vec<UniversalImageOccurrenceV2>,
    occurrence: UniversalImageOccurrenceV2,
) -> Result<()> {
    if output.len() >= MAX_IMAGE_OCCURRENCES {
        return Err(WellfriendError::ResourceLimit(
            "universal image occurrence count exceeds the governed limit 1000000".to_string(),
        ));
    }
    output.push(occurrence);
    Ok(())
}

fn content_numbers(operands: &[SpannedContentToken]) -> Vec<f64> {
    operands
        .iter()
        .filter_map(|operand| match &operand.token {
            ContentToken::Integer(value) => Some(*value as f64),
            ContentToken::Real(value) => Some(*value),
            _ => None,
        })
        .collect()
}

fn image_occurrence_id(
    revision: &str,
    page: usize,
    owner_number: u32,
    owner_generation: u16,
    start: usize,
    end: usize,
    resource_name: Option<&str>,
    invocation_path: &[VectorFormInvocation],
) -> String {
    let path = serde_json::to_vec(invocation_path).unwrap_or_default();
    stable_id(
        "image-occurrence-v2",
        &[
            revision.as_bytes(),
            &page.to_le_bytes(),
            &owner_number.to_le_bytes(),
            &owner_generation.to_le_bytes(),
            &start.to_le_bytes(),
            &end.to_le_bytes(),
            resource_name.unwrap_or("inline").as_bytes(),
            &path,
        ],
    )
}

fn image_bbox(matrix: UniversalImageMatrixV2) -> [f64; 4] {
    let points = [
        matrix.transform(0.0, 0.0),
        matrix.transform(1.0, 0.0),
        matrix.transform(0.0, 1.0),
        matrix.transform(1.0, 1.0),
    ];
    let mut bbox = [points[0][0], points[0][1], points[0][0], points[0][1]];
    for point in points.into_iter().skip(1) {
        bbox[0] = bbox[0].min(point[0]);
        bbox[1] = bbox[1].min(point[1]);
        bbox[2] = bbox[2].max(point[0]);
        bbox[3] = bbox[3].max(point[1]);
    }
    bbox
}

fn image_matrix_from_object(object: Option<&PdfObject>) -> Option<UniversalImageMatrixV2> {
    let values = object?.as_array()?;
    if values.len() != 6 {
        return None;
    }
    let mut numbers = [0.0; 6];
    for (target, value) in numbers.iter_mut().zip(values) {
        *target = value.as_number()?;
    }
    Some(UniversalImageMatrixV2 {
        a: numbers[0],
        b: numbers[1],
        c: numbers[2],
        d: numbers[3],
        e: numbers[4],
        f: numbers[5],
    })
}

fn resolve_universal_dict(
    object: Option<&PdfObject>,
    reader: &crate::reader::PdfReader,
) -> Option<PdfDictionary> {
    match object? {
        PdfObject::Dictionary(dict) => Some(dict.clone()),
        reference @ PdfObject::Reference { .. } => {
            reader.resolve(reference.clone()).ok()?.as_dict().cloned()
        }
        _ => None,
    }
}

fn image_filter_names(dict: &PdfDictionary) -> Vec<String> {
    let object = dict.get("Filter").or_else(|| dict.get("F"));
    match object {
        Some(PdfObject::Name(name)) => vec![name.clone()],
        Some(PdfObject::Array(items)) => items
            .iter()
            .filter_map(PdfObject::as_name)
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

fn image_color_space(dict: &PdfDictionary) -> Option<String> {
    match dict.get("ColorSpace").or_else(|| dict.get("CS")) {
        Some(PdfObject::Name(name)) => Some(name.clone()),
        Some(value) => Some(format!("{value:?}")),
        None => None,
    }
}

fn inline_image_metadata(
    parameters: &[SpannedContentToken],
) -> (Option<u32>, Option<u32>, Option<u8>, Option<String>, Vec<String>) {
    let mut width = None;
    let mut height = None;
    let mut bits = None;
    let mut color_space = None;
    let mut filters = Vec::new();
    for pair in parameters.windows(2) {
        let ContentToken::Name(key) = &pair[0].token else {
            continue;
        };
        match key.as_str() {
            "W" | "Width" => width = token_u32(&pair[1].token),
            "H" | "Height" => height = token_u32(&pair[1].token),
            "BPC" | "BitsPerComponent" => bits = token_u8(&pair[1].token),
            "CS" | "ColorSpace" => {
                color_space = match &pair[1].token {
                    ContentToken::Name(name) => Some(name.clone()),
                    _ => None,
                }
            }
            "F" | "Filter" => {
                if let ContentToken::Name(name) = &pair[1].token {
                    filters.push(name.clone());
                }
            }
            _ => {}
        }
    }
    (width, height, bits, color_space, filters)
}

fn token_u32(token: &ContentToken) -> Option<u32> {
    match token {
        ContentToken::Integer(value) => u32::try_from(*value).ok(),
        _ => None,
    }
}

fn token_u8(token: &ContentToken) -> Option<u8> {
    match token {
        ContentToken::Integer(value) => u8::try_from(*value).ok(),
        _ => None,
    }
}

fn pdf_u32(value: Option<i64>) -> Option<u32> {
    value.and_then(|value| u32::try_from(value).ok())
}

fn pdf_u8(value: Option<i64>) -> Option<u8> {
    value.and_then(|value| u8::try_from(value).ok())
}

struct UniversalObjectGraphPlanInternalV2 {
    candidates: Vec<UniversalCandidateV2>,
    read_set: Vec<String>,
    write_set: Vec<String>,
    report: Value,
}

pub fn inspect_universal_object_v2(
    input: &[u8],
    number: u32,
    generation: u16,
) -> Result<Value> {
    crate::cancel::check_current_cancel("universal object inspection")?;
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let object = engine.document().reader().get_object(number, generation)?;
    Ok(json!({
        "schema_version": UNIVERSAL_EDITING_SCHEMA_VERSION,
        "revision_id": revision_id(input),
        "number": number,
        "generation": generation,
        "fingerprint": universal_object_fingerprint_v2(&object),
        "kind": match &object {
            PdfObject::Stream { .. } => "stream",
            PdfObject::Dictionary(_) => "dictionary",
            PdfObject::Array(_) => "array",
            PdfObject::Reference { .. } => "reference",
            PdfObject::String(_) => "string",
            PdfObject::Name(_) => "name",
            PdfObject::Integer(_) | PdfObject::Real(_) => "number",
            PdfObject::Boolean(_) => "boolean",
            PdfObject::Null => "null",
        },
        "value": pdf_object_to_universal_value_v2(&object, 0)?,
        "debug_projection": format!("{object:?}"),
    }))
}

fn pdf_object_to_universal_value_v2(
    object: &PdfObject,
    depth: usize,
) -> Result<UniversalPdfValueV2> {
    if depth > 128 {
        return Err(WellfriendError::ResourceLimit(
            "universal object inspection exceeds nesting depth 128".to_string(),
        ));
    }
    Ok(match object {
        PdfObject::Null => UniversalPdfValueV2::Null,
        PdfObject::Boolean(value) => UniversalPdfValueV2::Boolean { value: *value },
        PdfObject::Integer(value) => UniversalPdfValueV2::Integer { value: *value },
        PdfObject::Real(value) => UniversalPdfValueV2::Real { value: *value },
        PdfObject::Name(value) => UniversalPdfValueV2::Name {
            value: value.clone(),
        },
        PdfObject::String(value) => UniversalPdfValueV2::String {
            value: value.clone(),
        },
        PdfObject::Array(items) => UniversalPdfValueV2::Array {
            items: items
                .iter()
                .map(|item| pdf_object_to_universal_value_v2(item, depth + 1))
                .collect::<Result<Vec<_>>>()?,
        },
        PdfObject::Dictionary(dictionary) => UniversalPdfValueV2::Dictionary {
            entries: dictionary
                .iter()
                .map(|(key, value)| {
                    Ok((
                        key.clone(),
                        pdf_object_to_universal_value_v2(value, depth + 1)?,
                    ))
                })
                .collect::<Result<BTreeMap<_, _>>>()?,
        },
        PdfObject::Stream { dict, raw } => UniversalPdfValueV2::Stream {
            entries: dict
                .iter()
                .filter(|(key, _)| key.as_str() != "Length")
                .map(|(key, value)| {
                    Ok((
                        key.clone(),
                        pdf_object_to_universal_value_v2(value, depth + 1)?,
                    ))
                })
                .collect::<Result<BTreeMap<_, _>>>()?,
            data: raw.clone(),
            flate_encode: false,
        },
        PdfObject::Reference { number, generation } => UniversalPdfValueV2::Reference {
            number: *number,
            generation: *generation,
        },
    })
}

fn plan_object_graph_edit_v2(
    input: &[u8],
    request: &UniversalObjectGraphEditRequestV2,
    revision: &str,
) -> Result<UniversalObjectGraphPlanInternalV2> {
    if request.mutations.is_empty() || request.mutations.len() > 4096 {
        return Err(WellfriendError::ResourceLimit(
            "universal object graph requires 1..=4096 mutations".to_string(),
        ));
    }
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let reader = engine.document().reader();
    let page_count = engine.page_count()?;
    if request
        .affected_pages
        .iter()
        .any(|page| *page == 0 || *page > page_count)
    {
        return Err(WellfriendError::invalid_input(
            "universal object graph affected page is outside the document",
        ));
    }
    if request.affected_pages.is_empty() && !request.acknowledge_global_resource_impact {
        return Err(WellfriendError::invalid_input(
            "universal object graph requires affected_pages or acknowledge_global_resource_impact=true",
        ));
    }
    let mut local_ids = BTreeSet::new();
    let mut existing_targets = BTreeSet::new();
    let mut total_payload_bytes = 0u64;
    for mutation in &request.mutations {
        match &mutation.target {
            UniversalObjectTargetV2::Existing {
                number,
                generation,
                expected_fingerprint,
            } => {
                if !existing_targets.insert((*number, *generation)) {
                    return Err(WellfriendError::invalid_input(
                        "universal object graph contains a duplicate existing-object target",
                    ));
                }
                let object = reader.get_object(*number, *generation)?;
                if expected_fingerprint.len() != 64
                    || universal_object_fingerprint_v2(&object) != *expected_fingerprint
                {
                    return Err(WellfriendError::invalid_input(format!(
                        "universal object graph fingerprint mismatch for {number} {generation} R"
                    )));
                }
            }
            UniversalObjectTargetV2::New { local_id } => {
                validate_local_object_id_v2(local_id)?;
                if !local_ids.insert(local_id.clone()) {
                    return Err(WellfriendError::invalid_input(
                        "universal object graph contains a duplicate local object id",
                    ));
                }
            }
        }
        validate_universal_pdf_value_v2(
            &mutation.value,
            reader,
            &mut total_payload_bytes,
            0,
        )?;
    }
    if total_payload_bytes > 2 * 1024 * 1024 * 1024 {
        return Err(WellfriendError::ResourceLimit(
            "universal object graph payload exceeds 2 GiB".to_string(),
        ));
    }
    for mutation in &request.mutations {
        validate_local_references_v2(&mutation.value, &local_ids, 0)?;
    }
    let mut candidates = Vec::with_capacity(request.mutations.len());
    let mut read_set = Vec::new();
    let mut write_set = Vec::new();
    for (index, mutation) in request.mutations.iter().enumerate() {
        let serialized = serde_json::to_vec(mutation).map_err(json_error)?;
        let candidate_id = stable_id(
            "object-graph-mutation-v2",
            &[revision.as_bytes(), &index.to_le_bytes(), serialized.as_slice()],
        );
        let (source_identity, shared_resource) = match &mutation.target {
            UniversalObjectTargetV2::Existing {
                number,
                generation,
                expected_fingerprint,
            } => {
                read_set.push(format!("object-{number}-{generation}:{expected_fingerprint}"));
                write_set.push(format!("object-{number}-{generation}"));
                (
                    json!({
                        "kind": "existing",
                        "number": number,
                        "generation": generation,
                        "fingerprint": expected_fingerprint,
                    }),
                    true,
                )
            }
            UniversalObjectTargetV2::New { local_id } => {
                write_set.push(format!("new-object:{local_id}"));
                (json!({"kind": "new", "local_id": local_id}), false)
            }
        };
        candidates.push(UniversalCandidateV2 {
            candidate_id,
            page: request.affected_pages.first().copied().unwrap_or(1),
            kind: "indirect_object_mutation".to_string(),
            source_identity,
            confidence: 1.0,
            exact: true,
            shared_resource,
            approval_reason: Some(
                "exact object-graph mutation may affect every resource consumer".to_string(),
            ),
        });
    }
    Ok(UniversalObjectGraphPlanInternalV2 {
        candidates,
        read_set,
        write_set,
        report: json!({
            "route": "canonical_incremental_indirect_object_graph",
            "mutation_count": request.mutations.len(),
            "new_object_count": local_ids.len(),
            "existing_object_count": existing_targets.len(),
            "payload_bytes": total_payload_bytes,
            "affected_pages": request.affected_pages,
            "global_resource_impact_acknowledged": request.acknowledge_global_resource_impact,
            "supports": ["patterns", "shadings", "annotation_appearances", "soft_masks", "optional_content", "custom_structure_trees", "vendor_extensions"],
        }),
    })
}

fn apply_object_graph_edit_v2(
    input: &[u8],
    request: &UniversalObjectGraphEditRequestV2,
    mode: UniversalMutationModeV2,
) -> Result<(Vec<u8>, Value, Vec<usize>, Vec<String>, Vec<String>)> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let reader = engine.document().reader();
    let mut next_number = next_universal_object_number(reader)?;
    let mut local_ids = request
        .mutations
        .iter()
        .filter_map(|mutation| match &mutation.target {
            UniversalObjectTargetV2::New { local_id } => Some(local_id.clone()),
            UniversalObjectTargetV2::Existing { .. } => None,
        })
        .collect::<Vec<_>>();
    local_ids.sort();
    local_ids.dedup();
    let mut local_references = BTreeMap::new();
    for local_id in local_ids {
        local_references.insert(
            local_id,
            allocate_universal_object_number(&mut next_number)?,
        );
    }
    let mut changed = Vec::with_capacity(request.mutations.len());
    for mutation in &request.mutations {
        let (number, generation) = match &mutation.target {
            UniversalObjectTargetV2::Existing {
                number,
                generation,
                expected_fingerprint,
            } => {
                let current = reader.get_object(*number, *generation)?;
                if universal_object_fingerprint_v2(&current) != *expected_fingerprint {
                    return Err(WellfriendError::invalid_input(format!(
                        "universal object graph target {number} {generation} R changed after planning"
                    )));
                }
                (*number, *generation)
            }
            UniversalObjectTargetV2::New { local_id } => (
                *local_references.get(local_id).ok_or_else(|| {
                    WellfriendError::MalformedPdf(
                        "universal object graph lost a local object assignment".to_string(),
                    )
                })?,
                0,
            ),
        };
        changed.push(IncrementalObject {
            number,
            generation,
            object: universal_pdf_value_to_object_v2(
                &mutation.value,
                &local_references,
                0,
            )?,
        });
    }
    let affected_objects = changed
        .iter()
        .map(|object| format!("object-{}-{}", object.number, object.generation))
        .collect::<Vec<_>>();
    let output = write_incremental_update(reader, changed)?;
    ContentEngine::open_bytes(output.clone())?;
    let affected_pages = if request.affected_pages.is_empty() {
        (1..=engine.page_count()?).collect::<Vec<_>>()
    } else {
        let mut pages = request.affected_pages.clone();
        pages.sort_unstable();
        pages.dedup();
        pages
    };
    Ok((
        output,
        json!({
            "operation": "replace_indirect_object_graph",
            "mutation_mode": mode,
            "mutation_count": request.mutations.len(),
            "local_object_assignments": local_references,
            "affected_pages": affected_pages,
            "output_reopened": true,
            "cryptographic_signature_validity_claimed": false,
        }),
        affected_pages,
        affected_objects,
        Vec::new(),
    ))
}

fn universal_object_fingerprint_v2(object: &PdfObject) -> String {
    digest_hex(format!("{object:?}").as_bytes())
}

fn validate_local_object_id_v2(local_id: &str) -> Result<()> {
    if local_id.is_empty()
        || local_id.len() > 128
        || !local_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(WellfriendError::invalid_input(
            "universal object graph local_id must be 1..=128 ASCII identifier bytes",
        ));
    }
    Ok(())
}

fn validate_universal_pdf_value_v2(
    value: &UniversalPdfValueV2,
    reader: &crate::reader::PdfReader,
    total_payload_bytes: &mut u64,
    depth: usize,
) -> Result<()> {
    if depth > 128 {
        return Err(WellfriendError::ResourceLimit(
            "universal object graph exceeds nesting depth 128".to_string(),
        ));
    }
    match value {
        UniversalPdfValueV2::Real { value } if !value.is_finite() => {
            return Err(WellfriendError::invalid_input(
                "universal PDF real values must be finite",
            ));
        }
        UniversalPdfValueV2::Name { value } => validate_pdf_name_v2(value, "PDF name")?,
        UniversalPdfValueV2::String { value } => {
            *total_payload_bytes = total_payload_bytes
                .checked_add(value.len() as u64)
                .ok_or_else(|| {
                    WellfriendError::ResourceLimit(
                        "universal object graph payload length overflow".to_string(),
                    )
                })?;
        }
        UniversalPdfValueV2::Array { items } => {
            if items.len() > 1_000_000 {
                return Err(WellfriendError::ResourceLimit(
                    "universal PDF array exceeds 1000000 items".to_string(),
                ));
            }
            for item in items {
                validate_universal_pdf_value_v2(item, reader, total_payload_bytes, depth + 1)?;
            }
        }
        UniversalPdfValueV2::Dictionary { entries }
        | UniversalPdfValueV2::Stream { entries, .. } => {
            if entries.len() > 1_000_000 {
                return Err(WellfriendError::ResourceLimit(
                    "universal PDF dictionary exceeds 1000000 entries".to_string(),
                ));
            }
            for (key, item) in entries {
                validate_pdf_name_v2(key, "PDF dictionary key")?;
                validate_universal_pdf_value_v2(item, reader, total_payload_bytes, depth + 1)?;
            }
            if let UniversalPdfValueV2::Stream { data, .. } = value {
                *total_payload_bytes = total_payload_bytes
                    .checked_add(data.len() as u64)
                    .ok_or_else(|| {
                        WellfriendError::ResourceLimit(
                            "universal object graph payload length overflow".to_string(),
                        )
                    })?;
            }
        }
        UniversalPdfValueV2::Reference { number, generation } => {
            reader.get_object(*number, *generation)?;
        }
        UniversalPdfValueV2::LocalReference { local_id } => {
            validate_local_object_id_v2(local_id)?;
        }
        _ => {}
    }
    Ok(())
}

fn validate_local_references_v2(
    value: &UniversalPdfValueV2,
    local_ids: &BTreeSet<String>,
    depth: usize,
) -> Result<()> {
    if depth > 128 {
        return Err(WellfriendError::ResourceLimit(
            "universal object graph exceeds nesting depth 128".to_string(),
        ));
    }
    match value {
        UniversalPdfValueV2::LocalReference { local_id } if !local_ids.contains(local_id) => {
            return Err(WellfriendError::invalid_input(format!(
                "universal object graph local reference '{local_id}' has no matching new-object mutation"
            )));
        }
        UniversalPdfValueV2::Array { items } => {
            for item in items {
                validate_local_references_v2(item, local_ids, depth + 1)?;
            }
        }
        UniversalPdfValueV2::Dictionary { entries }
        | UniversalPdfValueV2::Stream { entries, .. } => {
            for item in entries.values() {
                validate_local_references_v2(item, local_ids, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn universal_pdf_value_to_object_v2(
    value: &UniversalPdfValueV2,
    local_references: &BTreeMap<String, u32>,
    depth: usize,
) -> Result<PdfObject> {
    if depth > 128 {
        return Err(WellfriendError::ResourceLimit(
            "universal object graph exceeds nesting depth 128".to_string(),
        ));
    }
    Ok(match value {
        UniversalPdfValueV2::Null => PdfObject::Null,
        UniversalPdfValueV2::Boolean { value } => PdfObject::Boolean(*value),
        UniversalPdfValueV2::Integer { value } => PdfObject::Integer(*value),
        UniversalPdfValueV2::Real { value } => {
            if !value.is_finite() {
                return Err(WellfriendError::invalid_input(
                    "universal PDF real values must be finite",
                ));
            }
            PdfObject::Real(*value)
        }
        UniversalPdfValueV2::Name { value } => {
            validate_pdf_name_v2(value, "PDF name")?;
            PdfObject::Name(value.clone())
        }
        UniversalPdfValueV2::String { value } => PdfObject::String(value.clone()),
        UniversalPdfValueV2::Array { items } => PdfObject::Array(
            items
                .iter()
                .map(|item| universal_pdf_value_to_object_v2(item, local_references, depth + 1))
                .collect::<Result<Vec<_>>>()?,
        ),
        UniversalPdfValueV2::Dictionary { entries } => {
            let mut dictionary = PdfDictionary::empty();
            for (key, item) in entries {
                validate_pdf_name_v2(key, "PDF dictionary key")?;
                dictionary.insert(
                    key.clone(),
                    universal_pdf_value_to_object_v2(item, local_references, depth + 1)?,
                );
            }
            PdfObject::Dictionary(dictionary)
        }
        UniversalPdfValueV2::Stream {
            entries,
            data,
            flate_encode: use_flate,
        } => {
            let mut dictionary = PdfDictionary::empty();
            for (key, item) in entries {
                if key == "Length" || (*use_flate && matches!(key.as_str(), "Filter" | "DecodeParms")) {
                    continue;
                }
                validate_pdf_name_v2(key, "PDF stream dictionary key")?;
                dictionary.insert(
                    key.clone(),
                    universal_pdf_value_to_object_v2(item, local_references, depth + 1)?,
                );
            }
            let raw = if *use_flate {
                dictionary.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
                flate_encode_cancellable(data, 6)?
            } else {
                data.clone()
            };
            dictionary.insert("Length", PdfObject::Integer(raw.len() as i64));
            PdfObject::Stream {
                dict: dictionary,
                raw,
            }
        }
        UniversalPdfValueV2::Reference { number, generation } => PdfObject::Reference {
            number: *number,
            generation: *generation,
        },
        UniversalPdfValueV2::LocalReference { local_id } => PdfObject::Reference {
            number: *local_references.get(local_id).ok_or_else(|| {
                WellfriendError::invalid_input(format!(
                    "universal object graph local reference '{local_id}' was not assigned"
                ))
            })?,
            generation: 0,
        },
    })
}

fn document_subsystem_action_page_v2(
    action: Option<&crate::document_subsystems::DocumentSubsystemsAction>,
) -> Option<usize> {
    action
        .and_then(|action| serde_json::to_value(action).ok())
        .and_then(|value| value.get("page").and_then(Value::as_u64))
        .and_then(|page| usize::try_from(page).ok())
}

pub fn apply_universal_edit_v2(
    input: &[u8],
    plan: &UniversalEditPlanV2,
    approval: Option<&UniversalApprovalTokenV2>,
) -> Result<(Vec<u8>, UniversalEditResultV2)> {
    crate::cancel::check_current_cancel("universal editing apply entry")?;
    if !matches!(
        plan.policy.output_security,
        UniversalOutputSecurityPolicyV2::Unencrypted
    ) {
        return Err(WellfriendError::invalid_input(
            "universal edit plan requires apply_universal_edit_v2_with_output_security and apply-only credentials",
        ));
    }
    apply_universal_edit_v2_inner(input, plan, approval)
}

/// Preserve the exact transport container for a typed no-change result when a
/// binding planned against decrypted/canonicalized bytes. This prevents a
/// refused or target-not-found edit from accidentally returning plaintext in
/// place of the caller's encrypted original.
pub fn preserve_universal_no_change_transport_v2(
    original: &[u8],
    planning_input: &[u8],
    output: Vec<u8>,
    report: &mut UniversalEditResultV2,
) -> Vec<u8> {
    if report.changed || original == planning_input {
        return output;
    }
    let logical_report = std::mem::replace(&mut report.operation_report, Value::Null);
    report.operation_report = json!({
        "operation": logical_report,
        "transport_no_change": {
            "returned_exact_original_container": true,
            "planning_snapshot_was_decrypted_or_canonicalized": true,
            "returned_sha256": digest_hex(original),
        }
    });
    report.output_revision_id = revision_id(original);
    report.inverse = json!({
        "kind": "identity",
        "input_sha256": digest_hex(original),
        "output_sha256": digest_hex(original),
        "no_change_proof": true,
    });
    original.to_vec()
}

pub fn apply_universal_edit_v2_with_output_security(
    input: &[u8],
    plan: &UniversalEditPlanV2,
    approval: Option<&UniversalApprovalTokenV2>,
    credentials: &UniversalOutputSecurityCredentialsV2,
) -> Result<(Vec<u8>, UniversalEditResultV2)> {
    crate::cancel::check_current_cancel("universal secured editing apply entry")?;
    let UniversalOutputSecurityPolicyV2::Standard {
        algorithm,
        permissions,
        encrypt_metadata,
    } = plan.policy.output_security
    else {
        return Err(WellfriendError::invalid_input(
            "universal secured apply requires output_security.kind=standard in the immutable plan",
        ));
    };
    let (plaintext, mut result) = apply_universal_edit_v2_inner(input, plan, approval)?;
    crate::cancel::check_current_cancel("universal secured editing encryption")?;
    if !result.changed {
        return Ok((plaintext, result));
    }
    let algorithm = match algorithm {
        UniversalStandardEncryptionAlgorithmV2::Rc4_128
        | UniversalStandardEncryptionAlgorithmV2::Aes128 => {
            return Err(WellfriendError::UnsupportedFeature(
                "universal editing refuses legacy RC4-128/AES-128 output until independent cross-reader interoperability is qualified"
                    .to_string(),
            ))
        }
        UniversalStandardEncryptionAlgorithmV2::Aes256 => crate::EncryptAlgorithm::Aes256,
        UniversalStandardEncryptionAlgorithmV2::Aes256Gcm => {
            crate::EncryptAlgorithm::Aes256Gcm
        }
    };
    let params = crate::EncryptParams {
        user_password: credentials.user_password.clone(),
        owner_password: credentials.owner_password.clone(),
        permissions,
        algorithm,
        encrypt_metadata,
    };
    let plaintext_engine = ContentEngine::open_bytes(plaintext.clone())?;
    let encrypted = crate::structural::encrypt(&plaintext_engine, &params)?;
    crate::cancel::check_current_cancel("universal secured editing reopen")?;
    let reopened = ContentEngine::open_bytes_with_password(
        encrypted.clone(),
        credentials.user_password.as_slice(),
    )?;
    let required_profiles = required_conformance_profiles_v2(input, &plan.policy)?;
    if required_profiles
        .iter()
        .any(|profile| *profile == UniversalConformanceProfileV2::PdfUa1)
    {
        let pdfua = crate::compliance::validate_pdfua(reopened.document())?;
        if !pdfua.compliant {
            let current_snapshot = build_document_snapshot(input, None)?;
            let mut no_change = no_change_result(plan, &current_snapshot.revision_id);
            no_change.outcome = UniversalEditOutcomeV2::PolicyDenied;
            no_change.issues.push(json!({
                "code": "encrypted_output_pdfua_gate_failed",
                "message": "encrypted edited bytes failed the required PDF/UA validation; original bytes returned unchanged",
                "no_change_proof": true,
            }));
            no_change.conformance_impact = json!({
                "decision": "encrypted_edited_bytes_withheld",
                "pdfua": pdfua,
                "external_certification_claimed": false,
            });
            return Ok((input.to_vec(), no_change));
        }
    }
    let plaintext_revision = result.output_revision_id.clone();
    let encrypted_revision = revision_id(&encrypted);
    result.output_revision_id = encrypted_revision.clone();
    result.transaction_id = stable_id(
        "transaction-v2-secured",
        &[plan.plan_id.as_bytes(), encrypted_revision.as_bytes()],
    );
    let previous_report = std::mem::replace(&mut result.operation_report, Value::Null);
    result.operation_report = json!({
        "operation": previous_report,
        "output_security": {
            "status": "standard_security_handler_reencrypted",
            "algorithm": plan.policy.output_security,
            "permissions": permissions,
            "encrypt_metadata": encrypt_metadata,
            "plaintext_revision_id": plaintext_revision,
            "encrypted_revision_id": encrypted_revision,
            "credentials_serialized_or_reported": false,
            "output_reopened_with_user_credential": true,
        }
    });
    result.inverse = json!({
        "kind": "exact_preimage_restore",
        "input_sha256": digest_hex(input),
        "output_sha256": digest_hex(&encrypted),
        "preimage_retention_required": true,
    });
    Ok((encrypted, result))
}

fn apply_universal_edit_v2_inner(
    input: &[u8],
    plan: &UniversalEditPlanV2,
    approval: Option<&UniversalApprovalTokenV2>,
) -> Result<(Vec<u8>, UniversalEditResultV2)> {
    crate::cancel::check_current_cancel("universal editing apply snapshot")?;
    let current_snapshot = build_document_snapshot(input, None)?;
    let policy_engine = ContentEngine::open_bytes(input.to_vec())?;
    let secure_policy = analyze_edit_policy(&policy_engine, SignatureEditOperation::ContentEdit)?;
    if current_snapshot.revision_id != plan.revision_id {
        return Err(WellfriendError::invalid_input(
            "universal editing stale_plan: input revision differs from the planned revision",
        ));
    }
    let expected_plan = plan_id(
        &plan.revision_id,
        &plan.requested_operation,
        &plan.execution_operation,
        &plan.policy,
    )?;
    if expected_plan != plan.plan_id {
        return Err(WellfriendError::invalid_input(
            "universal editing plan content does not match its plan_id",
        ));
    }
    crate::cancel::check_current_cancel("universal editing canonical plan recomputation")?;
    let canonical_plan = plan_universal_edit_v2(
        input,
        &UniversalEditRequestV2 {
            operation: plan.requested_operation.clone(),
            policy: plan.policy.clone(),
        },
    )?;
    let supplied_plan = serde_json::to_value(plan).map_err(json_error)?;
    let canonical_plan_value = serde_json::to_value(&canonical_plan).map_err(json_error)?;
    if supplied_plan != canonical_plan_value {
        return Err(WellfriendError::AuthenticationFailure(
            "universal editing supplied plan differs from the canonical plan recomputed for this revision"
                .to_string(),
        ));
    }
    let validated_approval = if plan.state == UniversalPlanStateV2::ApprovalRequired {
        let token = approval.ok_or_else(|| {
            WellfriendError::invalid_input(
                "universal editing plan requires a revision-bound approval token",
            )
        })?;
        validate_approval(plan, token)?;
        Some(token)
    } else {
        None
    };
    if matches!(
        plan.state,
        UniversalPlanStateV2::PolicyDenied
            | UniversalPlanStateV2::TargetNotFound
            | UniversalPlanStateV2::IrrecoverableInput
    ) {
        return Ok((
            input.to_vec(),
            no_change_result(plan, &current_snapshot.revision_id),
        ));
    }
    enforce_universal_signature_policy(&secure_policy, plan.policy.mutation_mode)?;

    crate::cancel::check_current_cancel("universal editing operation dispatch")?;
    let applied = match &plan.execution_operation {
        UniversalEditOperationV2::Text { request } => {
            let mut effective_request = request.clone();
            effective_request.signature_policy_override = plan.policy.mutation_mode
                == UniversalMutationModeV2::AuthorizedRewrite;
            let selected_candidate_id = validated_approval
                .and_then(|token| token.decision.selected_candidate_ids.first())
                .map(String::as_str)
                .or_else(|| {
                    (plan.selected_candidate_ids.len() == 1)
                        .then(|| plan.selected_candidate_ids[0].as_str())
                });
            if let Some(candidate_id) = selected_candidate_id {
                let candidate = plan
                    .candidates
                    .iter()
                    .find(|candidate| candidate.candidate_id == candidate_id)
                    .ok_or_else(|| {
                        WellfriendError::invalid_input(
                            "universal text apply selected candidate is outside the canonical plan",
                        )
                    })?;
                bind_text_candidate_to_request(&mut effective_request, candidate)?;
            }
            if effective_request.font_policy == "allow_substitute" {
                let approved_font = validated_approval
                    .and_then(|token| token.decision.approved_font.as_deref())
                    .ok_or_else(|| {
                        WellfriendError::invalid_input(
                            "universal text apply requires the revision-bound approved font",
                        )
                    })?;
                effective_request.font_policy = format!("approved_substitute:{approved_font}");
            }
            apply_scene_text_transaction(input, &effective_request).map(|(bytes, report)| {
                (
                    bytes,
                    serde_json::to_value(&report).unwrap_or(Value::Null),
                    report.affected_pages,
                    report.affected_objects,
                    report.cloned_resources,
                )
            })
        }
        UniversalEditOperationV2::Image { request } => {
            let mut effective_request = request.clone();
            if effective_request.occurrence_id.is_none() {
                effective_request.occurrence_id = validated_approval
                    .and_then(|token| token.decision.selected_candidate_ids.first())
                    .cloned()
                    .or_else(|| {
                        (plan.selected_candidate_ids.len() == 1)
                            .then(|| plan.selected_candidate_ids[0].clone())
                    });
            }
            apply_image_edit(input, &effective_request, plan.policy.mutation_mode)
        }
        UniversalEditOperationV2::Vector { request } => edit_vector_object(
            input,
            request.page,
            &request.stable_id,
            request.operation.clone(),
            &VectorEditOptions {
                signature_policy_override: plan.policy.mutation_mode
                    == UniversalMutationModeV2::AuthorizedRewrite,
                deterministic: true,
                shared_form_policy: request.shared_resource_policy.into(),
            },
        )
        .map(|(bytes, report)| {
            let affected = report
                .after
                .as_ref()
                .map(|object| {
                    vec![format!(
                        "object-{}-{}",
                        object.provenance.object_number, object.provenance.generation
                    )]
                })
                .unwrap_or_else(|| plan.write_set.clone());
            (
                bytes,
                serde_json::to_value(&report).unwrap_or(Value::Null),
                vec![request.page],
                affected,
                report.clone_graph,
            )
        }),
        UniversalEditOperationV2::StructureCorrection { request } => {
            let mut effective_request = request.text_edit.clone();
            effective_request.signature_policy_override = plan.policy.mutation_mode
                == UniversalMutationModeV2::AuthorizedRewrite;
            let selected_candidate_id = validated_approval
                .and_then(|token| token.decision.selected_candidate_ids.first())
                .map(String::as_str)
                .or_else(|| {
                    (plan.selected_candidate_ids.len() == 1)
                        .then(|| plan.selected_candidate_ids[0].as_str())
                });
            if let Some(candidate_id) = selected_candidate_id {
                let candidate = plan
                    .candidates
                    .iter()
                    .find(|candidate| candidate.candidate_id == candidate_id)
                    .ok_or_else(|| {
                        WellfriendError::invalid_input(
                            "universal structure correction selected candidate is outside the canonical plan",
                        )
                    })?;
                bind_text_candidate_to_request(&mut effective_request, candidate)?;
            }
            if effective_request.font_policy == "allow_substitute" {
                let approved_font = validated_approval
                    .and_then(|token| token.decision.approved_font.as_deref())
                    .ok_or_else(|| {
                        WellfriendError::invalid_input(
                            "universal structure correction requires the revision-bound approved font",
                        )
                    })?;
                effective_request.font_policy = format!("approved_substitute:{approved_font}");
            }
            let (text_output, report) =
                apply_scene_text_transaction(input, &effective_request)?;
            let mut affected_pages = report.affected_pages.clone();
            let mut affected_objects = report.affected_objects.clone();
            let cloned_resources = report.cloned_resources.clone();
            let (bytes, structure_report) = if request.repair_tagged_structure {
                let security_request = crate::document_security::DocumentSecurityRequest {
                    subsystem:
                        crate::document_security::DocumentSecuritySubsystem::AccessibilityRepair,
                    action: Some(
                        crate::document_security::DocumentSecurityAction::RepairAfterMutation {
                            mutation:
                                crate::document_security::AccessibilityMutationKind::TextEdit,
                            lang: request.structure_language.clone(),
                        },
                    ),
                    approved: true,
                    language: request.structure_language.clone(),
                    full_rewrite_acknowledged: plan.policy.mutation_mode
                        == UniversalMutationModeV2::AuthorizedRewrite,
                };
                let (repaired, structure) =
                    crate::document_security::apply_document_security(
                        &text_output,
                        &security_request,
                    )?;
                affected_pages.extend(structure.changed_pages.iter().copied());
                affected_objects.extend(structure.write_set.iter().cloned());
                (
                    repaired,
                    serde_json::to_value(structure).map_err(json_error)?,
                )
            } else {
                (text_output, json!({"status": "caller_owned_object_graph"}))
            };
            affected_pages.sort_unstable();
            affected_pages.dedup();
            affected_objects.sort();
            affected_objects.dedup();
            Ok((
                bytes,
                json!({
                    "semantic_node_id": request.semantic_node_id,
                    "accepted_relationships": request.accepted_relationships,
                    "transaction": &report,
                    "structure_repair": structure_report,
                }),
                affected_pages,
                affected_objects,
                cloned_resources,
            ))
        }
        UniversalEditOperationV2::DocumentSubsystem { request } => {
            let mut effective_request = request.clone();
            effective_request.approved = true;
            crate::document_subsystems::apply_document_subsystems(input, &effective_request).map(
                |(bytes, report)| {
                    let affected_pages = report.changed_pages.clone();
                    (
                        bytes,
                        serde_json::to_value(&report).unwrap_or(Value::Null),
                        affected_pages,
                        plan.write_set.clone(),
                        Vec::new(),
                    )
                },
            )
        }
        UniversalEditOperationV2::DocumentSecurity { request } => {
            let mut effective_request = request.clone();
            effective_request.approved = true;
            if plan.policy.mutation_mode == UniversalMutationModeV2::AuthorizedRewrite {
                effective_request.full_rewrite_acknowledged = true;
            }
            crate::document_security::apply_document_security(input, &effective_request).map(
                |(bytes, report)| {
                    let affected_pages = report.changed_pages.clone();
                    (
                        bytes,
                        serde_json::to_value(&report).unwrap_or(Value::Null),
                        affected_pages,
                        report.write_set.clone(),
                        Vec::new(),
                    )
                },
            )
        }
        UniversalEditOperationV2::ObjectGraph { request } => {
            apply_object_graph_edit_v2(input, request, plan.policy.mutation_mode)
        }
    };

    let (mut output, mut operation_report, affected_pages, affected_objects, cloned_resources) =
        match applied {
            Ok(value) => value,
            Err(error) if matches!(error, WellfriendError::UnsupportedFeature(_)) => {
                let mut result = no_change_result(plan, &current_snapshot.revision_id);
                result.outcome = UniversalEditOutcomeV2::ApprovalRequired;
                result.issues.push(json!({
                    "code": "additional_reconstruction_decision_required",
                    "message": error.to_string(),
                    "no_change_proof": true,
                }));
                return Ok((input.to_vec(), result));
            }
            Err(error) => return Err(error),
        };
    crate::cancel::check_current_cancel("universal editing post-mutation")?;
    if universal_input_recovery_state(input)["strict_open"] == Value::Bool(false) {
        if !plan.policy.allow_deterministic_repair
            || plan.policy.mutation_mode != UniversalMutationModeV2::AuthorizedRewrite
        {
            return Err(WellfriendError::UnsupportedFeature(
                "universal editing recovered input requires approved deterministic full-rewrite normalization"
                    .to_string(),
            ));
        }
        crate::cancel::check_current_cancel("universal editing deterministic repair")?;
        let mutated_revision = revision_id(&output);
        output = crate::repair_pdf(output, b"")?;
        crate::reader::PdfReader::from_bytes_strict(output.clone()).map_err(|error| {
            WellfriendError::MalformedPdf(format!(
                "universal editing deterministic repair output failed strict reopen: {error}"
            ))
        })?;
        operation_report = json!({
            "operation": operation_report,
            "deterministic_input_normalization": {
                "status": "applied_after_source_mutation",
                "mutated_revision_id": mutated_revision,
                "normalized_revision_id": revision_id(&output),
                "authorized_full_rewrite": true,
            }
        });
    }
    crate::cancel::check_current_cancel("universal editing output reopen")?;
    ContentEngine::open_bytes(output.clone())?;
    let required_conformance_profiles =
        required_conformance_profiles_v2(input, &plan.policy)?;
    crate::cancel::check_current_cancel("universal editing conformance validation")?;
    let (conformance_passed, conformance_validation) =
        validate_universal_output_conformance_v2(&output, &required_conformance_profiles)?;
    if !conformance_passed {
        let mut result = no_change_result(plan, &current_snapshot.revision_id);
        result.outcome = UniversalEditOutcomeV2::PolicyDenied;
        result.conformance_impact = json!({
            "requested_guarantee": "preserve_declared_conformance",
            "decision": "edited_bytes_withheld",
            "validation": conformance_validation,
            "external_certification_claimed": false,
        });
        result.issues.push(json!({
            "code": "post_edit_conformance_gate_failed",
            "message": "edited bytes failed or could not conclusively pass a required standards profile; original bytes returned unchanged",
            "no_change_proof": true,
        }));
        return Ok((input.to_vec(), result));
    }
    if !required_conformance_profiles.is_empty() {
        operation_report = json!({
            "operation": operation_report,
            "post_edit_conformance_validation": &conformance_validation,
        });
    }
    let result_conformance_impact = if required_conformance_profiles.is_empty() {
        plan.conformance_impact.clone()
    } else {
        json!({
            "requested_guarantee": "preserve_declared_conformance",
            "decision": "built_in_profile_gate_passed",
            "validation": conformance_validation,
            "external_certification_claimed": false,
        })
    };
    let output_revision = revision_id(&output);
    let transaction_id = stable_id(
        "transaction-v2",
        &[plan.plan_id.as_bytes(), output_revision.as_bytes()],
    );
    let output_sha256 = digest_hex(&output);
    Ok((
        output,
        UniversalEditResultV2 {
            schema_version: UNIVERSAL_EDITING_SCHEMA_VERSION.to_string(),
            plan_id: plan.plan_id.clone(),
            transaction_id,
            outcome: UniversalEditOutcomeV2::Applied,
            changed: true,
            input_revision_id: plan.revision_id.clone(),
            output_revision_id: output_revision,
            affected_pages,
            affected_objects,
            cloned_resources,
            operation_report,
            render_invalidation: json!({
                "policy": "exact_source_dependencies_then_dirty_regions",
                "read_set": plan.read_set,
                "write_set": plan.write_set,
            }),
            signature_impact: plan.signature_impact.clone(),
            conformance_impact: result_conformance_impact,
            inverse: json!({
                "kind": "exact_preimage_restore",
                "input_sha256": digest_hex(input),
                "output_sha256": output_sha256,
                "preimage_retention_required": true,
            }),
            issues: Vec::new(),
        },
    ))
}

pub(crate) fn apply_image_edit(
    input: &[u8],
    request: &UniversalImageEditRequestV2,
    mode: UniversalMutationModeV2,
) -> Result<(Vec<u8>, Value, Vec<usize>, Vec<String>, Vec<String>)> {
    validate_image_replacement(&request.replacement)?;
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let mut matches = universal_image_occurrences_v2(input, &[request.page])?
        .into_iter()
        .filter(|candidate| {
            request.occurrence_id.as_deref().is_none_or(|id| candidate.occurrence_id == id)
                && request
                    .object_number
                    .is_none_or(|number| {
                        candidate.object_number == Some(number)
                            && candidate.generation == Some(request.generation)
                    })
                && request
                    .resource_name
                    .as_deref()
                    .is_none_or(|name| candidate.resource_name.as_deref() == Some(name))
        })
        .collect::<Vec<_>>();
    matches.sort_by_key(|candidate| {
        (
            candidate.content_stream_index,
            candidate.owner_stream_object,
            candidate.operation_byte_start,
            candidate.occurrence_id.clone(),
        )
    });
    let selected_index = if request.occurrence_id.is_some() {
        0
    } else {
        request.occurrence_index
    };
    let selected = matches.get(selected_index).ok_or_else(|| {
        WellfriendError::UnsupportedFeature(
            "universal image edit cannot resolve the selected image occurrence".to_string(),
        )
    })?;
    if selected.inline
        || request.shared_resource_policy == UniversalSharedResourcePolicyV2::CloneOne
    {
        return apply_image_occurrence_clone(input, &engine, selected, request, mode);
    }
    let reader = engine.document().reader();
    let object_number = selected.object_number.ok_or_else(|| {
        WellfriendError::MalformedPdf("selected image definition has no object identity".to_string())
    })?;
    let generation = selected.generation.ok_or_else(|| {
        WellfriendError::MalformedPdf("selected image definition has no generation".to_string())
    })?;
    let original = reader.get_object(object_number, generation)?;
    let next_number = next_universal_object_number(reader)?;
    let (replacement, mut auxiliary_objects) =
        replacement_image_object(&original, &request.replacement, next_number)?;
    let mut changed = vec![IncrementalObject {
        number: object_number,
        generation,
        object: replacement,
    }];
    changed.append(&mut auxiliary_objects);
    // Incremental serialization is valid in both modes and is required here
    // because a typed color-space graph may add ICC/function objects.  A later
    // authorized normalization pass may still canonicalize the complete file.
    let output = write_incremental_update(reader, changed)?;
    ContentEngine::open_bytes(output.clone())?;
    // Definition-wide replacement may also be observed through annotation
    // appearances, patterns, or other resource graphs not owned by the
    // page/Form occurrence selector. Invalidate all pages instead of claiming
    // an incomplete reverse-reference closure.
    let affected_pages = (1..=engine.page_count()?).collect::<Vec<_>>();
    Ok((
        output,
        json!({
            "operation": "replace_image_definition",
            "page": request.page,
            "occurrence_id": selected.occurrence_id,
            "resource_name": selected.resource_name,
            "object_number": object_number,
            "generation": generation,
            "shared_resource_policy": request.shared_resource_policy,
            "render_invalidation_scope": "all_pages_conservative_for_definition_wide_edit",
            "mutation_mode": mode,
            "replacement": {
                "width": request.replacement.width,
                "height": request.replacement.height,
                "bits_per_component": request.replacement.bits_per_component,
                "color_space": request.replacement.color_space,
                "color_space_descriptor": request.replacement.color_space_descriptor,
                "encoding": request.replacement.encoding,
                "image_mask": request.replacement.image_mask,
                "decode": request.replacement.decode,
                "payload_sha256": digest_hex(&request.replacement.data),
                "soft_mask": request.replacement.soft_mask.as_ref().map(|mask| json!({
                    "width": mask.width,
                    "height": mask.height,
                    "bits_per_component": mask.bits_per_component,
                    "samples_sha256": digest_hex(&mask.samples),
                })),
            },
            "output_reopened": true,
            "cryptographic_signature_validity_claimed": false,
        }),
        affected_pages,
        vec![format!(
            "object-{}-{}",
            object_number, generation
        )],
        Vec::new(),
    ))
}

/// Resolve and decode one exact image paint occurrence. Unlike definition-only
/// image APIs this keeps the page/Form occurrence identity, so a scanned-page
/// reconstruction can clone one shared use without guessing by resource name,
/// dimensions, or pixel similarity.
pub(crate) fn decode_image_occurrence_v2(
    input: &[u8],
    page: usize,
    occurrence_id: &str,
) -> Result<(UniversalImageOccurrenceV2, RawImage)> {
    if occurrence_id.trim().is_empty() {
        return Err(WellfriendError::invalid_input(
            "universal image occurrence id must be nonempty",
        ));
    }
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let occurrence = universal_image_occurrences_v2(input, &[page])?
        .into_iter()
        .find(|candidate| candidate.occurrence_id == occurrence_id)
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(
                "universal image decode cannot resolve the selected occurrence".to_string(),
            )
        })?;
    let (
        mut reference,
        inline_decode_params,
        inline_color_space_object,
        inline_image_dictionary,
    ) = if occurrence.inline {
        let owner = engine.document().reader().get_object(
            occurrence.owner_stream_object,
            occurrence.owner_stream_generation,
        )?;
        let decoded = decode_stream_lossless_with_limits(
            &owner,
            engine.document().reader(),
            &DecodeLimits {
                max_decoded_bytes_per_stream: 512 * 1024 * 1024,
                ..DecodeLimits::default()
            },
        )?;
        if decoded.status != StreamDecodeStatus::Complete {
            return Err(WellfriendError::UnsupportedFeature(
                "universal inline image owner is not losslessly decodable".to_string(),
            ));
        }
        let source = decoded
            .data
            .get(occurrence.operation_byte_start..occurrence.operation_byte_end)
            .ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "universal inline image occurrence range is outside its owner stream"
                        .to_string(),
                )
            })?;
        let details = ImageLocator::inline_decode_details_from_source(page, source)?;
        (
            details.reference,
            details.decode_params,
            details.color_space_object,
            details.image_dictionary,
        )
    } else {
        (ImageReference {
            page_number: page,
            xobject_name: occurrence
                .resource_name
                .clone()
                .unwrap_or_else(|| "occurrence_image".to_string()),
            object_number: occurrence.object_number.ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "universal image occurrence has no object number".to_string(),
                )
            })?,
            generation_number: occurrence.generation.ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "universal image occurrence has no generation".to_string(),
                )
            })?,
            width: occurrence.width.ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "universal image occurrence has no decoded width".to_string(),
                )
            })?,
            height: occurrence.height.ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "universal image occurrence has no decoded height".to_string(),
                )
            })?,
            bits_per_component: occurrence.bits_per_component.unwrap_or(8),
            color_space: occurrence
                .color_space
                .clone()
                .unwrap_or_else(|| "DeviceRGB".to_string()),
            filter: occurrence.filters.clone(),
            is_inline: false,
            is_mask: false,
            is_smask: false,
            inline_data: None,
        }, Vec::new(), None, PdfDictionary::empty())
    };
    let raw = if reference.is_inline {
        let resources = image_occurrence_resources_v2(&engine, page, &occurrence)?;
        let resolved_color_space = resolve_inline_color_space_v2(
            inline_color_space_object.as_ref(),
            &resources,
            engine.document().reader(),
        )?;
        if let Some((family, _)) = resolved_color_space.as_ref() {
            reference.color_space = family.clone();
        }
        let inline = reference.inline_data.as_ref().ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "universal inline image occurrence has no captured payload".to_string(),
            )
        })?;
        let filters = inline.filters.iter().map(String::as_str).collect::<Vec<_>>();
        ImageDecoder::decode_inline_with_resolved_image_dictionary_and_param_array(
            &inline.bytes,
            reference.width,
            reference.height,
            inline.bits_per_component,
            &reference.color_space,
            resolved_color_space.as_ref().map(|(_, object)| object),
            &filters,
            &inline_decode_params,
            &inline_image_dictionary,
            &DecodeLimits {
                max_decoded_bytes_per_stream: 512 * 1024 * 1024,
                ..DecodeLimits::default()
            },
            Some(engine.document().reader()),
            crate::render::cmm::ColorTransformOptions::default(),
        )?
    } else {
        ImageDecoder::decode(&reference, engine.document().reader())?
    };
    if !raw.is_valid() || raw.bits_per_sample != 8 {
        return Err(WellfriendError::UnsupportedFeature(
            "universal image occurrence did not decode to valid 8-bit interleaved samples"
                .to_string(),
        ));
    }
    Ok((occurrence, raw))
}

fn image_occurrence_resources_v2(
    engine: &ContentEngine,
    page: usize,
    occurrence: &UniversalImageOccurrenceV2,
) -> Result<PdfDictionary> {
    let page = engine.document().get_page(page)?;
    let reader = engine.document().reader();
    let mut resources = page.resources.clone();
    for invocation in &occurrence.invocation_path {
        let form = reader.get_object(invocation.form_object, invocation.form_generation)?;
        let PdfObject::Stream { dict, .. } = form else {
            return Err(WellfriendError::MalformedPdf(format!(
                "universal image invocation Form {} {} R is not a stream",
                invocation.form_object, invocation.form_generation
            )));
        };
        if let Some(form_resources) = resolve_universal_dict(dict.get("Resources"), reader) {
            resources = form_resources;
        }
    }
    Ok(resources)
}

fn resolve_inline_color_space_v2(
    declared: Option<&PdfObject>,
    resources: &PdfDictionary,
    reader: &crate::reader::PdfReader,
) -> Result<Option<(String, PdfObject)>> {
    let Some(declared) = declared else {
        return Ok(None);
    };
    let object = match declared {
        PdfObject::Name(name)
            if !matches!(
                name.as_str(),
                "DeviceGray" | "G" | "DeviceRGB" | "RGB" | "DeviceCMYK" | "CMYK"
            ) =>
        {
            let color_spaces = resolve_universal_dict(resources.get("ColorSpace"), reader)
                .ok_or_else(|| {
                    WellfriendError::MalformedPdf(format!(
                        "universal inline image named color space /{name} has no active /ColorSpace resources"
                    ))
                })?;
            color_spaces.get(name).cloned().ok_or_else(|| {
                WellfriendError::MalformedPdf(format!(
                    "universal inline image named color space /{name} is missing"
                ))
            })?
        }
        object => object.clone(),
    };
    let resolved = match &object {
        reference @ PdfObject::Reference { .. } => reader.resolve(reference.clone())?,
        _ => object.clone(),
    };
    let name = match &resolved {
        PdfObject::Name(name) => name.as_str(),
        PdfObject::Array(items) => items
            .first()
            .and_then(PdfObject::as_name)
            .ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "universal inline image color-space array has no family name".to_string(),
                )
            })?,
        _ => {
            return Err(WellfriendError::MalformedPdf(
                "universal inline image color-space resource is not a name or array".to_string(),
            ))
        }
    };
    let family = match name {
        "G" => "DeviceGray",
        "RGB" => "DeviceRGB",
        "CMYK" => "DeviceCMYK",
        other => other,
    };
    Ok(Some((family.to_string(), object)))
}

fn apply_image_occurrence_clone(
    input: &[u8],
    engine: &ContentEngine,
    selected: &UniversalImageOccurrenceV2,
    request: &UniversalImageEditRequestV2,
    mode: UniversalMutationModeV2,
) -> Result<(Vec<u8>, Value, Vec<usize>, Vec<String>, Vec<String>)> {
    let reader = engine.document().reader();
    let page = engine.document().get_page(selected.page)?;
    let mut next_number = next_universal_object_number(reader)?;
    let image_number = allocate_universal_object_number(&mut next_number)?;
    let (image_object, mut changed) = match selected.object_number.zip(selected.generation) {
        Some((number, generation)) => replacement_image_object(
            &reader.get_object(number, generation)?,
            &request.replacement,
            next_number,
        )?,
        None => replacement_image_object_from_dict(
            &PdfDictionary::empty(),
            &request.replacement,
            next_number,
        )?,
    };
    if let Some(number) = changed.iter().map(|object| object.number).max() {
        next_number = number.checked_add(1).ok_or_else(|| {
            WellfriendError::ResourceLimit(
                "universal image auxiliary object space exhausted".to_string(),
            )
        })?;
    }
    changed.insert(0, IncrementalObject {
        number: image_number,
        generation: 0,
        object: image_object,
    });
    let resource_sets = effective_resource_chain(reader, &page.resources, &selected.invocation_path)?;
    let mut clone_graph = Vec::new();

    let child_number = if selected.invocation_path.is_empty() {
        image_number
    } else {
        let leaf_resources = resource_sets.last().cloned().unwrap_or_else(|| page.resources.clone());
        let leaf_name = unique_xobject_name(reader, &leaf_resources, image_number, "UxI")?;
        let leaf_resources = resources_with_xobject(
            reader,
            &leaf_resources,
            &leaf_name,
            image_number,
        )?;
        let leaf_number = allocate_universal_object_number(&mut next_number)?;
        changed.push(IncrementalObject {
            number: leaf_number,
            generation: 0,
            object: cloned_stream_with_patch(
                reader,
                selected.owner_stream_object,
                selected.owner_stream_generation,
                selected.operation_byte_start..selected.operation_byte_end,
                format!("/{leaf_name} Do").as_bytes(),
                Some(leaf_resources),
            )?,
        });
        clone_graph.push(format!(
            "image:{} 0 R -> form:{} 0 R /{}; source owner {} {} R retained",
            image_number,
            leaf_number,
            leaf_name,
            selected.owner_stream_object,
            selected.owner_stream_generation
        ));
        let mut child_number = leaf_number;
        for (index, invocation) in selected
            .invocation_path
            .iter()
            .enumerate()
            .skip(1)
            .rev()
        {
            let owner_resources = resource_sets
                .get(index)
                .cloned()
                .unwrap_or_else(|| page.resources.clone());
            let child_name = unique_xobject_name(reader, &owner_resources, child_number, "UxF")?;
            let owner_resources = resources_with_xobject(
                reader,
                &owner_resources,
                &child_name,
                child_number,
            )?;
            let parent_number = allocate_universal_object_number(&mut next_number)?;
            changed.push(IncrementalObject {
                number: parent_number,
                generation: 0,
                object: cloned_stream_with_patch(
                    reader,
                    invocation.owner_stream_object,
                    invocation.owner_stream_generation,
                    invocation.owner_operation_byte_start..invocation.owner_operation_byte_end,
                    format!("/{child_name} Do").as_bytes(),
                    Some(owner_resources),
                )?,
            });
            clone_graph.push(format!(
                "form:{} {} R occurrence -> /{} {} 0 R; cloned owner as {} 0 R",
                invocation.form_object,
                invocation.form_generation,
                child_name,
                child_number,
                parent_number
            ));
            child_number = parent_number;
        }
        child_number
    };

    let (outer_range, outer_owner, outer_generation) = selected
        .invocation_path
        .first()
        .map(|invocation| {
            (
                invocation.owner_operation_byte_start..invocation.owner_operation_byte_end,
                invocation.owner_stream_object,
                invocation.owner_stream_generation,
            )
        })
        .unwrap_or((
            selected.operation_byte_start..selected.operation_byte_end,
            selected.owner_stream_object,
            selected.owner_stream_generation,
        ));
    let outer_name = unique_xobject_name(reader, &page.resources, child_number, "UxP")?;
    let page_resources = resources_with_xobject(
        reader,
        &page.resources,
        &outer_name,
        child_number,
    )?;
    let page_stream_number = next_number;
    let page_stream_object = cloned_stream_with_patch(
        reader,
        outer_owner,
        outer_generation,
        outer_range,
        format!("/{outer_name} Do").as_bytes(),
        None,
    )?;
    changed.push(IncrementalObject {
        number: page_stream_number,
        generation: 0,
        object: page_stream_object,
    });
    let page_object = reader.get_object(page.object_number, page.generation_number)?;
    let mut page_dict = page_object.as_dict().cloned().ok_or_else(|| {
        WellfriendError::MalformedPdf("universal image page object is not a dictionary".to_string())
    })?;
    page_dict.insert("Resources", PdfObject::Dictionary(page_resources));
    replace_page_content_reference(
        &mut page_dict,
        selected.content_stream_index,
        outer_owner,
        outer_generation,
        page_stream_number,
    )?;
    changed.push(IncrementalObject {
        number: page.object_number,
        generation: page.generation_number,
        object: PdfObject::Dictionary(page_dict),
    });
    clone_graph.push(format!(
        "page:{} content[{}] {} {} R -> {} 0 R /{}; source stream retained",
        selected.page,
        selected.content_stream_index,
        outer_owner,
        outer_generation,
        page_stream_number,
        outer_name
    ));
    let affected_objects = changed
        .iter()
        .map(|object| format!("object-{}-{}", object.number, object.generation))
        .collect::<Vec<_>>();
    let output = write_incremental_update(reader, changed)?;
    ContentEngine::open_bytes(output.clone())?;
    Ok((
        output,
        json!({
            "operation": if selected.inline { "promote_and_replace_inline_image_occurrence" } else { "clone_and_replace_image_occurrence" },
            "occurrence_id": selected.occurrence_id,
            "page": selected.page,
            "source_range": [selected.operation_byte_start, selected.operation_byte_end],
            "image_object": [image_number, 0],
            "page_content_clone": [page_stream_number, 0],
            "form_clone_count": clone_graph.len().saturating_sub(1),
            "mutation_mode": mode,
            "original_pdf_prefix_preserved": output.starts_with(input),
            "source_definitions_retained": true,
            "output_reopened": true,
            "cryptographic_signature_validity_claimed": false,
        }),
        vec![selected.page],
        affected_objects,
        clone_graph,
    ))
}

fn effective_resource_chain(
    reader: &crate::reader::PdfReader,
    page_resources: &PdfDictionary,
    path: &[VectorFormInvocation],
) -> Result<Vec<PdfDictionary>> {
    let mut output = vec![page_resources.clone()];
    let mut current = page_resources.clone();
    for invocation in path {
        let object = reader.get_object(invocation.form_object, invocation.form_generation)?;
        let PdfObject::Stream { dict, .. } = object else {
            return Err(WellfriendError::MalformedPdf(
                "universal image invocation does not resolve to a Form stream".to_string(),
            ));
        };
        current = resolve_universal_dict(dict.get("Resources"), reader)
            .unwrap_or_else(|| current.clone());
        output.push(current.clone());
    }
    Ok(output)
}

fn unique_xobject_name(
    reader: &crate::reader::PdfReader,
    resources: &PdfDictionary,
    seed: u32,
    prefix: &str,
) -> Result<String> {
    let xobjects = resolve_universal_dict(resources.get("XObject"), reader)
        .unwrap_or_else(PdfDictionary::empty);
    let mut index = 0u32;
    loop {
        let name = if index == 0 {
            format!("{prefix}{seed}")
        } else {
            format!("{prefix}{seed}_{index}")
        };
        if !xobjects.contains_key(&name) {
            return Ok(name);
        }
        index = index.checked_add(1).ok_or_else(|| {
            WellfriendError::ResourceLimit("universal image resource-name space exhausted".to_string())
        })?;
    }
}

fn resources_with_xobject(
    reader: &crate::reader::PdfReader,
    resources: &PdfDictionary,
    name: &str,
    object_number: u32,
) -> Result<PdfDictionary> {
    let mut output = resources.clone();
    let mut xobjects = resolve_universal_dict(output.get("XObject"), reader)
        .unwrap_or_else(PdfDictionary::empty);
    if xobjects.contains_key(name) {
        return Err(WellfriendError::MalformedPdf(format!(
            "universal image resource collision for /{name}"
        )));
    }
    xobjects.insert(
        name,
        PdfObject::Reference {
            number: object_number,
            generation: 0,
        },
    );
    output.insert("XObject", PdfObject::Dictionary(xobjects));
    Ok(output)
}

fn cloned_stream_with_patch(
    reader: &crate::reader::PdfReader,
    object_number: u32,
    generation: u16,
    range: std::ops::Range<usize>,
    replacement: &[u8],
    resources: Option<PdfDictionary>,
) -> Result<PdfObject> {
    let object = reader.get_object(object_number, generation)?;
    let PdfObject::Stream { mut dict, .. } = object.clone() else {
        return Err(WellfriendError::MalformedPdf(
            "universal image source owner is not a stream".to_string(),
        ));
    };
    let decoded = decode_stream_lossless_with_limits(
        &object,
        reader,
        &DecodeLimits {
            max_decoded_bytes_per_stream: 512 * 1024 * 1024,
            ..DecodeLimits::default()
        },
    )?;
    if decoded.status != StreamDecodeStatus::Complete {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "universal image source owner {object_number} {generation} R is not losslessly decodable"
        )));
    }
    if range.start > range.end || range.end > decoded.data.len() {
        return Err(WellfriendError::MalformedPdf(
            "universal image source range is outside its decoded stream".to_string(),
        ));
    }
    let mut data = decoded.data;
    data.splice(range, replacement.iter().copied());
    if let Some(resources) = resources {
        dict.insert("Resources", PdfObject::Dictionary(resources));
    }
    let raw = flate_encode_cancellable(&data, 6)?;
    dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
    for key in ["DecodeParms", "DP", "F", "FFilter", "FDecodeParms"] {
        dict.remove(key);
    }
    dict.insert("Length", PdfObject::Integer(raw.len() as i64));
    Ok(PdfObject::Stream { dict, raw })
}

fn replace_page_content_reference(
    page: &mut PdfDictionary,
    stream_index: usize,
    expected_number: u32,
    expected_generation: u16,
    replacement_number: u32,
) -> Result<()> {
    let replacement = PdfObject::Reference {
        number: replacement_number,
        generation: 0,
    };
    match page.get_mut("Contents") {
        Some(PdfObject::Reference { number, generation })
            if stream_index == 0
                && *number == expected_number
                && *generation == expected_generation =>
        {
            *number = replacement_number;
            *generation = 0;
            Ok(())
        }
        Some(PdfObject::Array(items)) => {
            let target = items.get_mut(stream_index).ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "universal image content stream index is outside /Contents".to_string(),
                )
            })?;
            if target.as_reference() != Some((expected_number, expected_generation)) {
                return Err(WellfriendError::MalformedPdf(
                    "universal image /Contents identity differs from occurrence provenance".to_string(),
                ));
            }
            *target = replacement;
            Ok(())
        }
        _ => Err(WellfriendError::MalformedPdf(
            "universal image page /Contents is missing or not an indirect stream reference"
                .to_string(),
        )),
    }
}

fn replacement_image_object(
    original: &PdfObject,
    replacement: &UniversalImageReplacementV2,
    next_object_number: u32,
) -> Result<(PdfObject, Vec<IncrementalObject>)> {
    let PdfObject::Stream { dict, .. } = original else {
        return Err(WellfriendError::MalformedPdf(
            "selected image XObject is not a stream".to_string(),
        ));
    };
    if dict.get_name("Subtype") != Some("Image") {
        return Err(WellfriendError::MalformedPdf(
            "selected XObject is not an image".to_string(),
        ));
    }
    let old_width = dict.get_integer("Width").or_else(|| dict.get_integer("W"));
    let old_height = dict.get_integer("Height").or_else(|| dict.get_integer("H"));
    let old_bits = dict
        .get_integer("BitsPerComponent")
        .or_else(|| dict.get_integer("BPC"));
    let mut source_dict = dict.clone();
    if replacement.preserve_masks
        && (dict.contains_key("Mask") || dict.contains_key("SMask"))
        && (old_width != Some(i64::from(replacement.width))
            || old_height != Some(i64::from(replacement.height)))
    {
        return Err(WellfriendError::UnsupportedFeature(
            "replacement image dimensions differ; associated masks cannot be preserved without resampling approval"
                .to_string(),
        ));
    }
    if replacement.preserve_masks {
        if let Some(PdfObject::Array(mask)) = dict.get("Mask") {
            let new_channels = usize::from(image_color_channels(replacement)?);
            if mask.len() != new_channels.saturating_mul(2) {
                return Err(WellfriendError::UnsupportedFeature(
                    "replacement image component count differs from the color-key mask; supply a regenerated mask through the object-graph transaction"
                        .to_string(),
                ));
            }
            if old_bits != Some(i64::from(replacement.bits_per_component)) {
                let old_bits = old_bits.and_then(|value| u8::try_from(value).ok()).ok_or_else(|| {
                    WellfriendError::UnsupportedFeature(
                        "color-key mask bit depth is missing; exact threshold regeneration is unavailable"
                            .to_string(),
                    )
                })?;
                source_dict.insert(
                    "Mask",
                    rescale_color_key_mask_v2(mask, old_bits, replacement.bits_per_component)?,
                );
            }
        }
    }
    replacement_image_object_from_dict(&source_dict, replacement, next_object_number)
}

fn rescale_color_key_mask_v2(mask: &[PdfObject], old_bits: u8, new_bits: u8) -> Result<PdfObject> {
    if !matches!(old_bits, 1 | 2 | 4 | 8 | 16)
        || !matches!(new_bits, 1 | 2 | 4 | 8 | 16)
    {
        return Err(WellfriendError::invalid_input(
            "color-key mask bit depth must be 1, 2, 4, 8, or 16",
        ));
    }
    let old_max = (1u64 << old_bits).saturating_sub(1);
    let new_max = (1u64 << new_bits).saturating_sub(1);
    if old_max == 0 || new_max == 0 {
        return Err(WellfriendError::invalid_input(
            "color-key mask bit depth must be positive",
        ));
    }
    let mut scaled = Vec::with_capacity(mask.len());
    for threshold in mask {
        let value = match threshold {
            PdfObject::Integer(value) => *value,
            _ => {
                return Err(WellfriendError::MalformedPdf(
                    "image color-key /Mask thresholds must be integers".to_string(),
                ));
            }
        };
        if value < 0 || value as u64 > old_max {
            return Err(WellfriendError::MalformedPdf(
                "image color-key /Mask threshold is outside the source sample range".to_string(),
            ));
        }
        let numerator = (value as u64)
            .checked_mul(new_max)
            .and_then(|product| product.checked_add(old_max / 2))
            .ok_or_else(|| {
                WellfriendError::ResourceLimit(
                    "image color-key /Mask threshold scaling overflow".to_string(),
                )
            })?;
        scaled.push(PdfObject::Integer((numerator / old_max) as i64));
    }
    Ok(PdfObject::Array(scaled))
}

fn replacement_image_object_from_dict(
    dict: &PdfDictionary,
    replacement: &UniversalImageReplacementV2,
    next_object_number: u32,
) -> Result<(PdfObject, Vec<IncrementalObject>)> {
    let mut output_dict = dict.clone();
    for key in [
        "W", "H", "BPC", "CS", "Filter", "F", "DecodeParms", "DP", "Decode", "D",
        "SMaskInData", "ImageMask", "IM", "Length",
    ] {
        output_dict.remove(key);
    }
    output_dict.insert("Type", PdfObject::Name("XObject".to_string()));
    output_dict.insert("Subtype", PdfObject::Name("Image".to_string()));
    output_dict.insert("Width", PdfObject::Integer(i64::from(replacement.width)));
    output_dict.insert("Height", PdfObject::Integer(i64::from(replacement.height)));
    output_dict.insert(
        "BitsPerComponent",
        PdfObject::Integer(i64::from(replacement.bits_per_component)),
    );
    let mut next_object_number = next_object_number;
    let mut auxiliary_objects = Vec::new();
    if replacement.image_mask {
        output_dict.insert("ImageMask", PdfObject::Boolean(true));
    } else {
        let color_space = if let Some(descriptor) = replacement.color_space_descriptor.as_ref() {
            build_image_color_space_v2(
                descriptor,
                &mut next_object_number,
                &mut auxiliary_objects,
                0,
            )?
        } else {
            PdfObject::Name(replacement.color_space.clone())
        };
        output_dict.insert("ColorSpace", color_space);
    }
    if let Some(decode) = replacement.decode.as_ref() {
        output_dict.insert("Decode", numbers(decode));
    }
    if replacement.soft_mask.is_some() || !replacement.preserve_masks {
        output_dict.remove("Mask");
        output_dict.remove("SMask");
        output_dict.remove("Matte");
    }
    if let Some(mask) = replacement.soft_mask.as_ref() {
        let mask_number = allocate_universal_object_number(&mut next_object_number)?;
        let encoded = flate_encode_cancellable(&mask.samples, 6)?;
        let mut mask_dict = PdfDictionary::empty();
        mask_dict.insert("Type", PdfObject::Name("XObject".to_string()));
        mask_dict.insert("Subtype", PdfObject::Name("Image".to_string()));
        mask_dict.insert("Width", PdfObject::Integer(i64::from(mask.width)));
        mask_dict.insert("Height", PdfObject::Integer(i64::from(mask.height)));
        mask_dict.insert(
            "BitsPerComponent",
            PdfObject::Integer(i64::from(mask.bits_per_component)),
        );
        mask_dict.insert("ColorSpace", PdfObject::Name("DeviceGray".to_string()));
        mask_dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
        mask_dict.insert("Length", PdfObject::Integer(encoded.len() as i64));
        auxiliary_objects.push(IncrementalObject {
            number: mask_number,
            generation: 0,
            object: PdfObject::Stream {
                dict: mask_dict,
                raw: encoded,
            },
        });
        output_dict.insert(
            "SMask",
            PdfObject::Reference {
                number: mask_number,
                generation: 0,
            },
        );
    }
    let raw = match replacement.encoding {
        UniversalImageEncodingV2::RawSamples => {
            output_dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
            flate_encode_cancellable(&replacement.data, 6)?
        }
        UniversalImageEncodingV2::Flate => {
            output_dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
            replacement.data.clone()
        }
        UniversalImageEncodingV2::Jpeg => {
            output_dict.insert("Filter", PdfObject::Name("DCTDecode".to_string()));
            replacement.data.clone()
        }
        UniversalImageEncodingV2::Jpx => {
            output_dict.insert("Filter", PdfObject::Name("JPXDecode".to_string()));
            replacement.data.clone()
        }
    };
    Ok((
        PdfObject::Stream {
            dict: output_dict,
            raw,
        },
        auxiliary_objects,
    ))
}

fn validate_image_replacement(replacement: &UniversalImageReplacementV2) -> Result<()> {
    if replacement.width == 0 || replacement.height == 0 {
        return Err(WellfriendError::invalid_input(
            "replacement image dimensions must be positive",
        ));
    }
    let pixels = u64::from(replacement.width)
        .checked_mul(u64::from(replacement.height))
        .ok_or_else(|| WellfriendError::ResourceLimit("replacement image dimensions overflow".to_string()))?;
    if pixels > MAX_IMAGE_PIXELS {
        return Err(WellfriendError::ResourceLimit(format!(
            "replacement image has {pixels} pixels; maximum is {MAX_IMAGE_PIXELS}"
        )));
    }
    if !matches!(replacement.bits_per_component, 1 | 2 | 4 | 8 | 16) {
        return Err(WellfriendError::invalid_input(
            "replacement bits_per_component must be 1, 2, 4, 8, or 16",
        ));
    }
    if replacement.data.is_empty() || replacement.data.len() > MAX_IMAGE_PAYLOAD_BYTES {
        return Err(WellfriendError::ResourceLimit(
            "replacement image payload is empty or exceeds 2 GiB".to_string(),
        ));
    }
    if replacement.preserve_masks && replacement.soft_mask.is_some() {
        return Err(WellfriendError::invalid_input(
            "replacement preserve_masks and explicit soft_mask are mutually exclusive",
        ));
    }
    if replacement.image_mask {
        if replacement.bits_per_component != 1 {
            return Err(WellfriendError::invalid_input(
                "replacement stencil image masks require bits_per_component=1",
            ));
        }
        if replacement.color_space_descriptor.is_some()
            || !replacement.color_space.trim().is_empty()
        {
            return Err(WellfriendError::invalid_input(
                "replacement stencil image masks cannot declare a color space",
            ));
        }
        if replacement.soft_mask.is_some() || replacement.preserve_masks {
            return Err(WellfriendError::invalid_input(
                "replacement stencil image masks cannot carry or preserve Mask/SMask entries",
            ));
        }
        if !matches!(
            replacement.encoding,
            UniversalImageEncodingV2::RawSamples | UniversalImageEncodingV2::Flate
        ) {
            return Err(WellfriendError::invalid_input(
                "replacement stencil image masks require raw or Flate-packed samples",
            ));
        }
    }
    if let Some(decode) = replacement.decode.as_ref() {
        let channels = usize::from(image_color_channels(replacement)?);
        if decode.len() != channels.saturating_mul(2)
            || decode.iter().any(|value| !value.is_finite())
        {
            return Err(WellfriendError::invalid_input(format!(
                "replacement Decode must contain exactly {} finite values",
                channels.saturating_mul(2)
            )));
        }
        if replacement.image_mask
            && !((decode[0] == 0.0 && decode[1] == 1.0)
                || (decode[0] == 1.0 && decode[1] == 0.0))
        {
            return Err(WellfriendError::invalid_input(
                "replacement stencil image-mask Decode must be [0 1] or [1 0]",
            ));
        }
    }
    if let Some(mask) = replacement.soft_mask.as_ref() {
        if mask.width != replacement.width || mask.height != replacement.height {
            return Err(WellfriendError::invalid_input(
                "replacement soft-mask dimensions must equal the primary image dimensions",
            ));
        }
        if !matches!(mask.bits_per_component, 1 | 2 | 4 | 8 | 16) {
            return Err(WellfriendError::invalid_input(
                "replacement soft-mask bits_per_component must be 1, 2, 4, 8, or 16",
            ));
        }
        let row_bits = u64::from(mask.width)
            .checked_mul(u64::from(mask.bits_per_component))
            .ok_or_else(|| {
                WellfriendError::ResourceLimit(
                    "replacement soft-mask row size overflow".to_string(),
                )
            })?;
        let expected = row_bits
            .div_ceil(8)
            .checked_mul(u64::from(mask.height))
            .ok_or_else(|| {
                WellfriendError::ResourceLimit(
                    "replacement soft-mask sample size overflow".to_string(),
                )
            })?;
        if expected != mask.samples.len() as u64 {
            return Err(WellfriendError::invalid_input(format!(
                "replacement soft-mask sample length is {}; expected {expected}",
                mask.samples.len()
            )));
        }
        if mask.samples.len() > MAX_IMAGE_PAYLOAD_BYTES {
            return Err(WellfriendError::ResourceLimit(
                "replacement soft-mask payload exceeds 2 GiB".to_string(),
            ));
        }
    }
    if let Some(descriptor) = replacement.color_space_descriptor.as_ref() {
        validate_finite_color_space_v2(descriptor)?;
        image_color_descriptor_channels_v2(descriptor, 0)?;
    }
    if replacement.encoding == UniversalImageEncodingV2::RawSamples {
        let bytes = expected_image_sample_bytes(replacement)?;
        if bytes != replacement.data.len() as u64 {
            return Err(WellfriendError::invalid_input(format!(
                "raw replacement sample length is {}; expected {bytes}",
                replacement.data.len()
            )));
        }
    }
    if replacement.encoding == UniversalImageEncodingV2::Flate {
        let expected = expected_image_sample_bytes(replacement)?;
        let decoded = crate::filters::flate_decode_capped(&replacement.data, expected)?;
        if decoded.len() as u64 != expected {
            return Err(WellfriendError::invalid_input(format!(
                "Flate replacement sample length is {}; expected {expected}",
                decoded.len()
            )));
        }
    }
    if replacement.encoding == UniversalImageEncodingV2::Jpeg {
        if replacement.bits_per_component != 8 {
            return Err(WellfriendError::invalid_input(
                "JPEG replacement images require bits_per_component=8",
            ));
        }
        let (width, height, channels) = ImageDecoder::jpeg_metadata(&replacement.data)?;
        validate_encoded_image_metadata(replacement, width, height, channels, Some(8), "JPEG")?;
    }
    if replacement.encoding == UniversalImageEncodingV2::Jpx {
        let (width, height, channels, bits) = jpeg2000_metadata(&replacement.data)?;
        validate_encoded_image_metadata(replacement, width, height, channels, bits, "JPEG2000")?;
    }
    Ok(())
}

fn image_color_channels(replacement: &UniversalImageReplacementV2) -> Result<u8> {
    if replacement.image_mask {
        return Ok(1);
    }
    if let Some(descriptor) = replacement.color_space_descriptor.as_ref() {
        return image_color_descriptor_channels_v2(descriptor, 0);
    }
    let color_space = replacement.color_space.as_str();
    match color_space {
        "DeviceGray" => Ok(1),
        "DeviceRGB" => Ok(3),
        "DeviceCMYK" => Ok(4),
        _ => Err(WellfriendError::invalid_input(
            "legacy replacement.color_space accepts DeviceGray, DeviceRGB, or DeviceCMYK; supply color_space_descriptor for calibrated, ICCBased, Indexed, Separation, or DeviceN samples",
        )),
    }
}

fn image_color_descriptor_channels_v2(
    descriptor: &UniversalImageColorSpaceV2,
    depth: usize,
) -> Result<u8> {
    if depth > 16 {
        return Err(WellfriendError::ResourceLimit(
            "replacement image color-space graph exceeds depth 16".to_string(),
        ));
    }
    match descriptor {
        UniversalImageColorSpaceV2::DeviceGray | UniversalImageColorSpaceV2::CalGray { .. } => {
            Ok(1)
        }
        UniversalImageColorSpaceV2::DeviceRgb
        | UniversalImageColorSpaceV2::CalRgb { .. }
        | UniversalImageColorSpaceV2::Lab { .. } => Ok(3),
        UniversalImageColorSpaceV2::DeviceCmyk => Ok(4),
        UniversalImageColorSpaceV2::IccBased { components, .. } => {
            if !(1..=15).contains(components) {
                return Err(WellfriendError::invalid_input(
                    "ICCBased replacement components must be in 1..=15",
                ));
            }
            Ok(*components)
        }
        UniversalImageColorSpaceV2::Indexed { .. }
        | UniversalImageColorSpaceV2::Separation { .. } => Ok(1),
        UniversalImageColorSpaceV2::DeviceN { colorants, .. } => u8::try_from(colorants.len())
            .ok()
            .filter(|count| (1..=32).contains(count))
            .ok_or_else(|| {
                WellfriendError::invalid_input(
                    "DeviceN replacement requires 1..=32 colorant names",
                )
            }),
    }
}

fn build_image_color_space_v2(
    descriptor: &UniversalImageColorSpaceV2,
    next_object_number: &mut u32,
    auxiliary_objects: &mut Vec<IncrementalObject>,
    depth: usize,
) -> Result<PdfObject> {
    if depth > 16 {
        return Err(WellfriendError::ResourceLimit(
            "replacement image color-space graph exceeds depth 16".to_string(),
        ));
    }
    validate_finite_color_space_v2(descriptor)?;
    match descriptor {
        UniversalImageColorSpaceV2::DeviceGray => {
            Ok(PdfObject::Name("DeviceGray".to_string()))
        }
        UniversalImageColorSpaceV2::DeviceRgb => {
            Ok(PdfObject::Name("DeviceRGB".to_string()))
        }
        UniversalImageColorSpaceV2::DeviceCmyk => {
            Ok(PdfObject::Name("DeviceCMYK".to_string()))
        }
        UniversalImageColorSpaceV2::CalGray {
            white_point,
            black_point,
            gamma,
        } => {
            let mut parameters = PdfDictionary::empty();
            parameters.insert("WhitePoint", numbers(white_point));
            if let Some(value) = black_point {
                parameters.insert("BlackPoint", numbers(value));
            }
            if let Some(value) = gamma {
                parameters.insert("Gamma", PdfObject::Real(*value));
            }
            Ok(PdfObject::Array(vec![
                PdfObject::Name("CalGray".to_string()),
                PdfObject::Dictionary(parameters),
            ]))
        }
        UniversalImageColorSpaceV2::CalRgb {
            white_point,
            black_point,
            gamma,
            matrix,
        } => {
            let mut parameters = PdfDictionary::empty();
            parameters.insert("WhitePoint", numbers(white_point));
            if let Some(value) = black_point {
                parameters.insert("BlackPoint", numbers(value));
            }
            if let Some(value) = gamma {
                parameters.insert("Gamma", numbers(value));
            }
            if let Some(value) = matrix {
                parameters.insert("Matrix", numbers(value));
            }
            Ok(PdfObject::Array(vec![
                PdfObject::Name("CalRGB".to_string()),
                PdfObject::Dictionary(parameters),
            ]))
        }
        UniversalImageColorSpaceV2::Lab {
            white_point,
            black_point,
            range,
        } => {
            let mut parameters = PdfDictionary::empty();
            parameters.insert("WhitePoint", numbers(white_point));
            if let Some(value) = black_point {
                parameters.insert("BlackPoint", numbers(value));
            }
            if let Some(value) = range {
                parameters.insert("Range", numbers(value));
            }
            Ok(PdfObject::Array(vec![
                PdfObject::Name("Lab".to_string()),
                PdfObject::Dictionary(parameters),
            ]))
        }
        UniversalImageColorSpaceV2::IccBased {
            components,
            profile,
            alternate,
            range,
        } => {
            image_color_descriptor_channels_v2(descriptor, depth)?;
            if profile.is_empty() || profile.len() > 128 * 1024 * 1024 {
                return Err(WellfriendError::ResourceLimit(
                    "ICCBased profile is empty or exceeds 128 MiB".to_string(),
                ));
            }
            let profile_components = icc_profile_components_v2(profile)?;
            if profile_components != *components {
                return Err(WellfriendError::invalid_input(format!(
                    "ICCBased profile declares {profile_components} input components but the color-space descriptor declares {components}"
                )));
            }
            if !range.is_empty() && range.len() != usize::from(*components) * 2 {
                return Err(WellfriendError::invalid_input(
                    "ICCBased Range must contain two numbers per component",
                ));
            }
            let mut dictionary = PdfDictionary::empty();
            dictionary.insert("N", PdfObject::Integer(i64::from(*components)));
            if let Some(alternate) = alternate {
                if !is_process_color_space_v2(alternate) {
                    return Err(WellfriendError::invalid_input(
                        "ICCBased Alternate must be a device, calibrated, Lab, or ICC process color space"
                            .to_string(),
                    ));
                }
                let alternate_components =
                    image_color_descriptor_channels_v2(alternate, depth + 1)?;
                if alternate_components != *components {
                    return Err(WellfriendError::invalid_input(format!(
                        "ICCBased Alternate has {alternate_components} components but the profile requires {components}"
                    )));
                }
                dictionary.insert(
                    "Alternate",
                    build_image_color_space_v2(
                        alternate,
                        next_object_number,
                        auxiliary_objects,
                        depth + 1,
                    )?,
                );
            }
            if !range.is_empty() {
                dictionary.insert("Range", numbers(range));
            }
            let compressed = flate_encode_cancellable(profile, 6)?;
            dictionary.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
            dictionary.insert("Length", PdfObject::Integer(compressed.len() as i64));
            let number = allocate_universal_object_number(next_object_number)?;
            auxiliary_objects.push(IncrementalObject {
                number,
                generation: 0,
                object: PdfObject::Stream {
                    dict: dictionary,
                    raw: compressed,
                },
            });
            Ok(PdfObject::Array(vec![
                PdfObject::Name("ICCBased".to_string()),
                PdfObject::Reference {
                    number,
                    generation: 0,
                },
            ]))
        }
        UniversalImageColorSpaceV2::Indexed {
            base,
            high_value,
            lookup,
        } => {
            if !is_process_color_space_v2(base) {
                return Err(WellfriendError::invalid_input(
                    "Indexed base must be a device, calibrated, Lab, or ICC process color space"
                        .to_string(),
                ));
            }
            let base_channels = usize::from(image_color_descriptor_channels_v2(base, depth + 1)?);
            let expected = (usize::from(*high_value) + 1)
                .checked_mul(base_channels)
                .ok_or_else(|| {
                    WellfriendError::ResourceLimit(
                        "Indexed color-space lookup length overflow".to_string(),
                    )
                })?;
            if lookup.len() != expected {
                return Err(WellfriendError::invalid_input(format!(
                    "Indexed color-space lookup contains {} bytes; expected {expected}",
                    lookup.len()
                )));
            }
            Ok(PdfObject::Array(vec![
                PdfObject::Name("Indexed".to_string()),
                build_image_color_space_v2(
                    base,
                    next_object_number,
                    auxiliary_objects,
                    depth + 1,
                )?,
                PdfObject::Integer(i64::from(*high_value)),
                PdfObject::String(lookup.clone()),
            ]))
        }
        UniversalImageColorSpaceV2::Separation {
            colorant,
            alternate,
            tint_transform,
        } => {
            validate_pdf_name_v2(colorant, "Separation colorant")?;
            if !is_process_color_space_v2(alternate) {
                return Err(WellfriendError::invalid_input(
                    "Separation alternate must be a device, calibrated, Lab, or ICC process color space"
                        .to_string(),
                ));
            }
            let alternate_channels = image_color_descriptor_channels_v2(alternate, depth + 1)?;
            Ok(PdfObject::Array(vec![
                PdfObject::Name("Separation".to_string()),
                PdfObject::Name(colorant.clone()),
                build_image_color_space_v2(
                    alternate,
                    next_object_number,
                    auxiliary_objects,
                    depth + 1,
                )?,
                build_pdf_function_v2(
                    tint_transform,
                    1,
                    alternate_channels,
                    next_object_number,
                    auxiliary_objects,
                    depth + 1,
                )?,
            ]))
        }
        UniversalImageColorSpaceV2::DeviceN {
            colorants,
            alternate,
            tint_transform,
            attributes,
        } => {
            let input_channels = image_color_descriptor_channels_v2(descriptor, depth)?;
            let unique_colorants = colorants.iter().collect::<BTreeSet<_>>();
            if unique_colorants.len() != colorants.len() {
                return Err(WellfriendError::invalid_input(
                    "DeviceN replacement colorant names must be unique",
                ));
            }
            for colorant in colorants {
                validate_pdf_name_v2(colorant, "DeviceN colorant")?;
            }
            if !is_process_color_space_v2(alternate) {
                return Err(WellfriendError::invalid_input(
                    "DeviceN alternate must be a device, calibrated, Lab, or ICC process color space"
                        .to_string(),
                ));
            }
            let alternate_channels = image_color_descriptor_channels_v2(alternate, depth + 1)?;
            let mut result = vec![
                PdfObject::Name("DeviceN".to_string()),
                PdfObject::Array(
                    colorants
                        .iter()
                        .cloned()
                        .map(PdfObject::Name)
                        .collect(),
                ),
                build_image_color_space_v2(
                    alternate,
                    next_object_number,
                    auxiliary_objects,
                    depth + 1,
                )?,
                build_pdf_function_v2(
                    tint_transform,
                    input_channels,
                    alternate_channels,
                    next_object_number,
                    auxiliary_objects,
                    depth + 1,
                )?,
            ];
            if !attributes.is_empty() {
                let local_references = BTreeMap::new();
                let mut dictionary = PdfDictionary::empty();
                for (key, value) in attributes {
                    validate_pdf_name_v2(key, "DeviceN attribute key")?;
                    dictionary.insert(
                        key.clone(),
                        universal_pdf_value_to_object_v2(value, &local_references, 0)?,
                    );
                }
                result.push(PdfObject::Dictionary(dictionary));
            }
            Ok(PdfObject::Array(result))
        }
    }
}

fn is_process_color_space_v2(descriptor: &UniversalImageColorSpaceV2) -> bool {
    matches!(
        descriptor,
        UniversalImageColorSpaceV2::DeviceGray
            | UniversalImageColorSpaceV2::DeviceRgb
            | UniversalImageColorSpaceV2::DeviceCmyk
            | UniversalImageColorSpaceV2::CalGray { .. }
            | UniversalImageColorSpaceV2::CalRgb { .. }
            | UniversalImageColorSpaceV2::Lab { .. }
            | UniversalImageColorSpaceV2::IccBased { .. }
    )
}

fn build_pdf_function_v2(
    function: &UniversalPdfFunctionV2,
    input_channels: u8,
    output_channels: u8,
    next_object_number: &mut u32,
    auxiliary_objects: &mut Vec<IncrementalObject>,
    depth: usize,
) -> Result<PdfObject> {
    if depth > 32 {
        return Err(WellfriendError::ResourceLimit(
            "replacement PDF function graph exceeds depth 32".to_string(),
        ));
    }
    match function {
        UniversalPdfFunctionV2::Exponential {
            domain,
            c0,
            c1,
            exponent,
        } => {
            if input_channels != 1
                || c0.len() != usize::from(output_channels)
                || c1.len() != usize::from(output_channels)
                || !domain.iter().chain(c0).chain(c1).all(|value| value.is_finite())
                || domain[0] >= domain[1]
                || !exponent.is_finite()
                || *exponent <= 0.0
            {
                return Err(WellfriendError::invalid_input(
                    "exponential tint transform requires one input, finite values, positive exponent, and one C0/C1 value per alternate component",
                ));
            }
            let mut dictionary = PdfDictionary::empty();
            dictionary.insert("FunctionType", PdfObject::Integer(2));
            dictionary.insert("Domain", numbers(domain));
            dictionary.insert("C0", numbers(c0));
            dictionary.insert("C1", numbers(c1));
            dictionary.insert("N", PdfObject::Real(*exponent));
            Ok(PdfObject::Dictionary(dictionary))
        }
        UniversalPdfFunctionV2::Sampled {
            domain,
            range,
            size,
            bits_per_sample,
            samples,
            encode,
            decode,
        } => {
            if domain.len() != usize::from(input_channels) * 2
                || range.len() != usize::from(output_channels) * 2
                || size.len() != usize::from(input_channels)
                || size.iter().any(|size| *size == 0)
                || !matches!(bits_per_sample, 1 | 2 | 4 | 8 | 12 | 16 | 24 | 32)
                || !domain.iter().chain(range).chain(encode).chain(decode).all(|v| v.is_finite())
                || (!encode.is_empty() && encode.len() != usize::from(input_channels) * 2)
                || (!decode.is_empty() && decode.len() != usize::from(output_channels) * 2)
                || domain.chunks_exact(2).any(|pair| pair[0] >= pair[1])
                || range.chunks_exact(2).any(|pair| pair[0] >= pair[1])
            {
                return Err(WellfriendError::invalid_input(
                    "sampled tint transform dimensions, ranges, or BitsPerSample are invalid",
                ));
            }
            let sample_values = size.iter().try_fold(1u64, |total, value| {
                total.checked_mul(u64::from(*value))
            }).and_then(|total| total.checked_mul(u64::from(output_channels))).ok_or_else(|| {
                WellfriendError::ResourceLimit("sampled tint transform size overflow".to_string())
            })?;
            let expected = sample_values
                .checked_mul(u64::from(*bits_per_sample))
                .map(|bits| bits.div_ceil(8))
                .ok_or_else(|| WellfriendError::ResourceLimit("sampled tint transform byte size overflow".to_string()))?;
            if expected != samples.len() as u64 || samples.len() > 128 * 1024 * 1024 {
                return Err(WellfriendError::invalid_input(format!(
                    "sampled tint transform contains {} bytes; expected {expected}", samples.len()
                )));
            }
            let mut dictionary = PdfDictionary::empty();
            dictionary.insert("FunctionType", PdfObject::Integer(0));
            dictionary.insert("Domain", numbers(domain));
            dictionary.insert("Range", numbers(range));
            dictionary.insert(
                "Size",
                PdfObject::Array(size.iter().map(|v| PdfObject::Integer(i64::from(*v))).collect()),
            );
            dictionary.insert("BitsPerSample", PdfObject::Integer(i64::from(*bits_per_sample)));
            if !encode.is_empty() {
                dictionary.insert("Encode", numbers(encode));
            }
            if !decode.is_empty() {
                dictionary.insert("Decode", numbers(decode));
            }
            let raw = flate_encode_cancellable(samples, 6)?;
            dictionary.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
            dictionary.insert("Length", PdfObject::Integer(raw.len() as i64));
            let number = allocate_universal_object_number(next_object_number)?;
            auxiliary_objects.push(IncrementalObject {
                number,
                generation: 0,
                object: PdfObject::Stream { dict: dictionary, raw },
            });
            Ok(PdfObject::Reference { number, generation: 0 })
        }
        UniversalPdfFunctionV2::Stitching {
            domain,
            range,
            functions,
            bounds,
            encode,
        } => {
            if input_channels != 1
                || !domain.iter().chain(range).chain(bounds).chain(encode).all(|v| v.is_finite())
                || domain[0] >= domain[1]
                || functions.is_empty()
                || functions.len() > 4_096
                || bounds.len() + 1 != functions.len()
                || encode.len() != functions.len() * 2
                || (!range.is_empty() && range.len() != usize::from(output_channels) * 2)
                || bounds.iter().any(|bound| *bound <= domain[0] || *bound >= domain[1])
                || bounds.windows(2).any(|pair| pair[0] >= pair[1])
            {
                return Err(WellfriendError::invalid_input(
                    "stitching tint transform requires one input, an ordered finite domain/bounds sequence, one more function than bounds, and two Encode values per function",
                ));
            }
            let mut children = Vec::with_capacity(functions.len());
            for child in functions {
                children.push(build_pdf_function_v2(
                    child,
                    1,
                    output_channels,
                    next_object_number,
                    auxiliary_objects,
                    depth + 1,
                )?);
            }
            let mut dictionary = PdfDictionary::empty();
            dictionary.insert("FunctionType", PdfObject::Integer(3));
            dictionary.insert("Domain", numbers(domain));
            if !range.is_empty() {
                dictionary.insert("Range", numbers(range));
            }
            dictionary.insert("Functions", PdfObject::Array(children));
            dictionary.insert("Bounds", numbers(bounds));
            dictionary.insert("Encode", numbers(encode));
            Ok(PdfObject::Dictionary(dictionary))
        }
        UniversalPdfFunctionV2::Calculator {
            domain,
            range,
            program,
        } => {
            if domain.len() != usize::from(input_channels) * 2
                || range.len() != usize::from(output_channels) * 2
                || !domain.iter().chain(range).all(|v| v.is_finite())
                || program.is_empty()
                || program.len() > 16 * 1024 * 1024
                || domain.chunks_exact(2).any(|pair| pair[0] >= pair[1])
                || range.chunks_exact(2).any(|pair| pair[0] >= pair[1])
                || !crate::render::function::validate_type4_program(program)
            {
                return Err(WellfriendError::invalid_input(
                    "calculator tint transform dimensions, ranges, or restricted PostScript program are invalid",
                ));
            }
            let mut dictionary = PdfDictionary::empty();
            dictionary.insert("FunctionType", PdfObject::Integer(4));
            dictionary.insert("Domain", numbers(domain));
            dictionary.insert("Range", numbers(range));
            dictionary.insert("Length", PdfObject::Integer(program.len() as i64));
            let number = allocate_universal_object_number(next_object_number)?;
            auxiliary_objects.push(IncrementalObject {
                number,
                generation: 0,
                object: PdfObject::Stream {
                    dict: dictionary,
                    raw: program.clone(),
                },
            });
            Ok(PdfObject::Reference { number, generation: 0 })
        }
    }
}

fn numbers(values: &[f64]) -> PdfObject {
    PdfObject::Array(values.iter().map(|value| PdfObject::Real(*value)).collect())
}

fn allocate_universal_object_number(next: &mut u32) -> Result<u32> {
    let number = *next;
    *next = next.checked_add(1).ok_or_else(|| {
        WellfriendError::ResourceLimit("universal PDF object number space exhausted".to_string())
    })?;
    Ok(number)
}

fn next_universal_object_number(reader: &crate::reader::PdfReader) -> Result<u32> {
    reader
        .object_ids()
        .into_iter()
        .map(|(number, _)| number)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| {
            WellfriendError::ResourceLimit(
                "universal PDF object number space is exhausted".to_string(),
            )
        })
}

fn validate_finite_color_space_v2(descriptor: &UniversalImageColorSpaceV2) -> Result<()> {
    let finite = match descriptor {
        UniversalImageColorSpaceV2::CalGray { white_point, black_point, gamma } => {
            white_point.iter().chain(black_point.iter().flatten()).chain(gamma).all(|v| v.is_finite())
        }
        UniversalImageColorSpaceV2::CalRgb { white_point, black_point, gamma, matrix } => {
            white_point.iter().chain(black_point.iter().flatten()).chain(gamma.iter().flatten()).chain(matrix.iter().flatten()).all(|v| v.is_finite())
        }
        UniversalImageColorSpaceV2::Lab { white_point, black_point, range } => {
            white_point.iter().chain(black_point.iter().flatten()).chain(range.iter().flatten()).all(|v| v.is_finite())
        }
        UniversalImageColorSpaceV2::IccBased { range, .. } => range.iter().all(|v| v.is_finite()),
        _ => true,
    };
    if !finite {
        return Err(WellfriendError::invalid_input(
            "replacement image color-space parameters must be finite",
        ));
    }
    match descriptor {
        UniversalImageColorSpaceV2::CalGray {
            white_point,
            black_point,
            gamma,
        } => {
            validate_calibrated_white_black_points_v2(white_point, black_point.as_ref())?;
            if gamma.is_some_and(|value| value <= 0.0) {
                return Err(WellfriendError::invalid_input(
                    "CalGray Gamma must be positive",
                ));
            }
        }
        UniversalImageColorSpaceV2::CalRgb {
            white_point,
            black_point,
            gamma,
            ..
        } => {
            validate_calibrated_white_black_points_v2(white_point, black_point.as_ref())?;
            if gamma.is_some_and(|values| values.iter().any(|value| *value <= 0.0)) {
                return Err(WellfriendError::invalid_input(
                    "CalRGB Gamma components must be positive",
                ));
            }
        }
        UniversalImageColorSpaceV2::Lab {
            white_point,
            black_point,
            range,
        } => {
            validate_calibrated_white_black_points_v2(white_point, black_point.as_ref())?;
            if range.is_some_and(|values| values[0] >= values[1] || values[2] >= values[3]) {
                return Err(WellfriendError::invalid_input(
                    "Lab Range minima must be lower than their maxima",
                ));
            }
        }
        UniversalImageColorSpaceV2::IccBased { range, .. } => {
            if range.chunks_exact(2).any(|pair| pair[0] >= pair[1]) {
                return Err(WellfriendError::invalid_input(
                    "ICCBased Range minima must be lower than their maxima",
                ));
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_calibrated_white_black_points_v2(
    white_point: &[f64; 3],
    black_point: Option<&[f64; 3]>,
) -> Result<()> {
    if white_point[0] <= 0.0
        || (white_point[1] - 1.0).abs() > 1e-9
        || white_point[2] <= 0.0
        || black_point.is_some_and(|values| values.iter().any(|value| *value < 0.0))
    {
        return Err(WellfriendError::invalid_input(
            "calibrated color-space WhitePoint requires positive X/Z and Y=1; BlackPoint components must be nonnegative",
        ));
    }
    Ok(())
}

fn icc_profile_components_v2(profile: &[u8]) -> Result<u8> {
    if profile.len() < 128 {
        return Err(WellfriendError::invalid_input(
            "ICCBased profile is shorter than the 128-byte ICC header",
        ));
    }
    let declared_size = u32::from_be_bytes(
        profile[0..4]
            .try_into()
            .map_err(|_| WellfriendError::invalid_input("ICCBased profile size is truncated"))?,
    ) as usize;
    if declared_size != profile.len() || &profile[36..40] != b"acsp" {
        return Err(WellfriendError::invalid_input(
            "ICCBased profile must have an exact declared byte length and an ICC 'acsp' signature",
        ));
    }
    let signature = &profile[16..20];
    let components = match signature {
        b"GRAY" => 1,
        b"RGB " | b"XYZ " | b"Lab " | b"Luv " | b"YCbr" | b"Yxy " => 3,
        b"CMYK" => 4,
        _ if &signature[1..4] == b"CLR" => match signature[0] {
            b'2'..=b'9' => signature[0] - b'0',
            b'A'..=b'F' => signature[0] - b'A' + 10,
            _ => 0,
        },
        _ => 0,
    };
    if components == 0 {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "ICCBased profile data color-space signature {:?} is not a governed 1..=15 component ICC signature",
            String::from_utf8_lossy(signature)
        )));
    }
    Ok(components)
}

fn validate_pdf_name_v2(name: &str, label: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 127
        || name.bytes().any(|byte| byte <= 0x20 || b"()<>[]{}/%#".contains(&byte))
    {
        return Err(WellfriendError::invalid_input(format!(
            "{label} is not a bounded literal PDF name"
        )));
    }
    Ok(())
}

fn required_conformance_profiles_v2(
    input: &[u8],
    policy: &UniversalEditPolicyV2,
) -> Result<Vec<UniversalConformanceProfileV2>> {
    let mut profiles = policy
        .required_conformance_profiles
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if policy.require_conformance_preservation && profiles.is_empty() {
        let engine = ContentEngine::open_bytes(input.to_vec())?;
        let options = crate::standards_engine::StandardsValidationOptions::default();
        for report in [
            crate::standards_engine::validate_pdfa_profile(&engine, &options)?,
            crate::standards_engine::validate_pdfua_profile(&engine, &options)?,
            crate::standards_engine::validate_pdfx_profile(&engine, &options)?,
        ] {
            if let Some(label) = report.detection.claimed_label.as_deref() {
                if let Some(profile) = universal_conformance_profile_from_label_v2(label) {
                    profiles.insert(profile);
                }
            }
        }
    }
    Ok(profiles.into_iter().collect())
}

fn universal_conformance_profile_from_label_v2(
    label: &str,
) -> Option<UniversalConformanceProfileV2> {
    let normalized = label
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>()
        .to_ascii_uppercase();
    match normalized.as_str() {
        "PDF/A-1B" => Some(UniversalConformanceProfileV2::PdfA1B),
        "PDF/A-2B" => Some(UniversalConformanceProfileV2::PdfA2B),
        "PDF/A-2A" => Some(UniversalConformanceProfileV2::PdfA2A),
        "PDF/A-3B" => Some(UniversalConformanceProfileV2::PdfA3B),
        "PDF/A-3A" => Some(UniversalConformanceProfileV2::PdfA3A),
        "PDF/UA-1" => Some(UniversalConformanceProfileV2::PdfUa1),
        "PDF/X-1A" | "PDF/X-1A:2001" => {
            Some(UniversalConformanceProfileV2::PdfX1A2001)
        }
        "PDF/X-3" | "PDF/X-3:2003" => Some(UniversalConformanceProfileV2::PdfX3_2003),
        "PDF/X-4" => Some(UniversalConformanceProfileV2::PdfX4),
        _ => None,
    }
}

fn validate_universal_output_conformance_v2(
    output: &[u8],
    profiles: &[UniversalConformanceProfileV2],
) -> Result<(bool, Value)> {
    if profiles.is_empty() {
        return Ok((true, json!({
            "status": "not_requested_or_no_declared_profile_detected",
            "profiles": [],
            "external_certification_claimed": false,
        })));
    }
    let engine = ContentEngine::open_bytes(output.to_vec())?;
    let mut all_passed = true;
    let mut reports = Vec::with_capacity(profiles.len());
    for profile in profiles {
        let (passed, validator, report) = match profile {
            UniversalConformanceProfileV2::PdfA1B
            | UniversalConformanceProfileV2::PdfA2B
            | UniversalConformanceProfileV2::PdfA2A
            | UniversalConformanceProfileV2::PdfA3B
            | UniversalConformanceProfileV2::PdfA3A => {
                let pdfa_profile = match profile {
                    UniversalConformanceProfileV2::PdfA1B => {
                        crate::compliance::PdfAProfile::PdfA1B
                    }
                    UniversalConformanceProfileV2::PdfA2B => {
                        crate::compliance::PdfAProfile::PdfA2B
                    }
                    UniversalConformanceProfileV2::PdfA2A => {
                        crate::compliance::PdfAProfile::PdfA2A
                    }
                    UniversalConformanceProfileV2::PdfA3B => {
                        crate::compliance::PdfAProfile::PdfA3B
                    }
                    UniversalConformanceProfileV2::PdfA3A => {
                        crate::compliance::PdfAProfile::PdfA3A
                    }
                    _ => unreachable!(),
                };
                let report = crate::compliance::validate_pdfa(engine.document(), pdfa_profile)?;
                (
                    report.compliant,
                    "compliance::validate_pdfa",
                    serde_json::to_value(report).map_err(json_error)?,
                )
            }
            UniversalConformanceProfileV2::PdfUa1 => {
                let report = crate::compliance::validate_pdfua(engine.document())?;
                (
                    report.compliant,
                    "compliance::validate_pdfua",
                    serde_json::to_value(report).map_err(json_error)?,
                )
            }
            UniversalConformanceProfileV2::PdfX1A2001
            | UniversalConformanceProfileV2::PdfX3_2003
            | UniversalConformanceProfileV2::PdfX4 => {
                let options = crate::standards_engine::StandardsValidationOptions::with_target(
                    profile.label(),
                );
                let report = crate::standards_engine::validate_pdfx_profile(&engine, &options)?;
                let passed = report.is_conformant();
                (
                    passed,
                    "standards_engine::validate_pdfx_profile",
                    serde_json::to_value(report).map_err(json_error)?,
                )
            }
        };
        all_passed &= passed;
        reports.push(json!({
            "profile": profile.label(),
            "passed": passed,
            "validator": validator,
            "report": report,
        }));
    }
    Ok((all_passed, json!({
        "status": if all_passed { "passed" } else { "failed_or_inconclusive" },
        "profiles": reports,
        "all_required_profiles_passed": all_passed,
        "external_certification_claimed": false,
    })))
}

fn expected_image_sample_bytes(replacement: &UniversalImageReplacementV2) -> Result<u64> {
    let channels = u64::from(image_color_channels(replacement)?);
    let row_bits = u64::from(replacement.width)
        .checked_mul(channels)
        .and_then(|value| value.checked_mul(u64::from(replacement.bits_per_component)))
        .ok_or_else(|| WellfriendError::ResourceLimit("replacement row size overflow".to_string()))?;
    row_bits
        .div_ceil(8)
        .checked_mul(u64::from(replacement.height))
        .ok_or_else(|| WellfriendError::ResourceLimit("replacement sample size overflow".to_string()))
}

fn validate_encoded_image_metadata(
    replacement: &UniversalImageReplacementV2,
    width: u32,
    height: u32,
    channels: u8,
    bits: Option<u8>,
    encoding: &str,
) -> Result<()> {
    let expected_channels = image_color_channels(replacement)?;
    if width != replacement.width || height != replacement.height {
        return Err(WellfriendError::invalid_input(format!(
            "{encoding} payload dimensions {width}x{height} differ from declared {}x{}",
            replacement.width, replacement.height
        )));
    }
    if channels != expected_channels {
        return Err(WellfriendError::invalid_input(format!(
            "{encoding} payload has {channels} components but {} requires {expected_channels}",
            replacement.color_space
        )));
    }
    if bits.is_some_and(|bits| bits != replacement.bits_per_component) {
        return Err(WellfriendError::invalid_input(format!(
            "{encoding} payload bit depth {:?} differs from declared {}",
            bits, replacement.bits_per_component
        )));
    }
    Ok(())
}

fn jpeg2000_metadata(data: &[u8]) -> Result<(u32, u32, u8, Option<u8>)> {
    const JP2_SIGNATURE: &[u8] = b"\x00\x00\x00\x0cjP  \r\n\x87\n";
    if data.starts_with(JP2_SIGNATURE) {
        return jp2_header_metadata(data).ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "JPEG2000 JP2 payload has no bounded ihdr image header".to_string(),
            )
        });
    }
    if data.starts_with(&[0xff, 0x4f]) {
        return j2k_codestream_metadata(data).ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "JPEG2000 codestream has no bounded SIZ marker".to_string(),
            )
        });
    }
    Err(WellfriendError::MalformedPdf(
        "JPEG2000 replacement is neither a JP2 file nor a raw J2K codestream".to_string(),
    ))
}

fn jp2_header_metadata(data: &[u8]) -> Option<(u32, u32, u8, Option<u8>)> {
    fn boxes(data: &[u8]) -> Option<(u32, u32, u8, Option<u8>)> {
        let mut cursor = 0usize;
        let mut image_header = None::<(u32, u32, u8, u8)>;
        let mut component_bits = None::<(u8, usize)>;
        while cursor.checked_add(8)? <= data.len() {
            let short = u32::from_be_bytes(data.get(cursor..cursor + 4)?.try_into().ok()?);
            let kind = data.get(cursor + 4..cursor + 8)?;
            let (header, length) = if short == 1 {
                let length = u64::from_be_bytes(data.get(cursor + 8..cursor + 16)?.try_into().ok()?);
                (16usize, usize::try_from(length).ok()?)
            } else if short == 0 {
                (8usize, data.len().saturating_sub(cursor))
            } else {
                (8usize, usize::try_from(short).ok()?)
            };
            if length < header || cursor.checked_add(length)? > data.len() {
                return None;
            }
            let payload = data.get(cursor + header..cursor + length)?;
            if kind == b"ihdr" && payload.len() >= 11 {
                let height = u32::from_be_bytes(payload[0..4].try_into().ok()?);
                let width = u32::from_be_bytes(payload[4..8].try_into().ok()?);
                let channels = u16::from_be_bytes(payload[8..10].try_into().ok()?);
                let encoded_bits = payload[10];
                image_header = Some((
                    width,
                    height,
                    u8::try_from(channels).ok()?,
                    encoded_bits,
                ));
            }
            if kind == b"bpcc" && !payload.is_empty() {
                let first = (payload[0] & 0x7f) + 1;
                if payload.iter().any(|value| (value & 0x7f) + 1 != first) {
                    return None;
                }
                component_bits = Some((first, payload.len()));
            }
            if kind == b"jp2h" {
                if let Some(metadata) = boxes(payload) {
                    return Some(metadata);
                }
            }
            cursor = cursor.checked_add(length)?;
        }
        let (width, height, channels, encoded_bits) = image_header?;
        let bits = if encoded_bits == 255 {
            let (bits, component_count) = component_bits?;
            if component_count != usize::from(channels) {
                return None;
            }
            bits
        } else {
            (encoded_bits & 0x7f) + 1
        };
        Some((width, height, channels, Some(bits)))
    }
    boxes(data)
}

fn j2k_codestream_metadata(data: &[u8]) -> Option<(u32, u32, u8, Option<u8>)> {
    let limit = data.len().min(4096);
    let marker = data[..limit]
        .windows(2)
        .position(|window| window == [0xff, 0x51])?;
    let length = usize::from(u16::from_be_bytes(
        data.get(marker + 2..marker + 4)?.try_into().ok()?,
    ));
    if length < 41 || marker.checked_add(2 + length)? > data.len() {
        return None;
    }
    let xsiz = u32::from_be_bytes(data.get(marker + 6..marker + 10)?.try_into().ok()?);
    let ysiz = u32::from_be_bytes(data.get(marker + 10..marker + 14)?.try_into().ok()?);
    let xosiz = u32::from_be_bytes(data.get(marker + 14..marker + 18)?.try_into().ok()?);
    let yosiz = u32::from_be_bytes(data.get(marker + 18..marker + 22)?.try_into().ok()?);
    let channels = u16::from_be_bytes(data.get(marker + 38..marker + 40)?.try_into().ok()?);
    let channels_usize = usize::from(channels);
    let components_end = marker.checked_add(40usize.checked_add(3usize.checked_mul(channels_usize)?)?)?;
    if components_end > marker.checked_add(2 + length)? {
        return None;
    }
    let first_bits = (*data.get(marker + 40)? & 0x7f) + 1;
    for component in 0..channels_usize {
        let ssiz = *data.get(marker + 40 + component * 3)?;
        if (ssiz & 0x7f) + 1 != first_bits {
            return None;
        }
    }
    Some((
        xsiz.checked_sub(xosiz)?,
        ysiz.checked_sub(yosiz)?,
        u8::try_from(channels).ok()?,
        Some(first_bits),
    ))
}

fn normalize_pages(requested: &[usize], page_count: usize, limit: usize) -> Result<Vec<usize>> {
    let mut pages = if requested.is_empty() {
        (1..=page_count.min(limit)).collect::<Vec<_>>()
    } else {
        requested.to_vec()
    };
    if pages.len() > limit {
        return Err(WellfriendError::ResourceLimit(format!(
            "universal analysis requested {} pages; maximum per request is {limit}",
            pages.len()
        )));
    }
    if pages.iter().any(|page| *page == 0 || *page > page_count) {
        return Err(WellfriendError::invalid_input(
            "universal analysis page is outside the document",
        ));
    }
    pages.sort_unstable();
    pages.dedup();
    Ok(pages)
}

fn capability(
    id: &str,
    status: UniversalCapabilityStatusV2,
    owner: &str,
    behavior: &str,
    approval_triggers: &[&str],
    policy_limits: &[&str],
) -> UniversalCapabilityV2 {
    UniversalCapabilityV2 {
        id: id.to_string(),
        status,
        owner: owner.to_string(),
        behavior: behavior.to_string(),
        approval_triggers: approval_triggers.iter().map(|value| (*value).to_string()).collect(),
        policy_limits: policy_limits.iter().map(|value| (*value).to_string()).collect(),
        source_implementation: "present_in_current_source_tree".to_string(),
        qualification_status: "not_executed_in_this_implementation_change; vps_corpus_gate_pending"
            .to_string(),
    }
}

fn logical_text_range_candidates_v2(
    input: &[u8],
    request: &SceneTextEditRequest,
    revision: &str,
    existing: &[UniversalCandidateV2],
) -> Result<Vec<UniversalCandidateV2>> {
    let model = analyze_multi_run_text_range(input, request.page)?;
    let scalar_len = model.logical_text.chars().count();
    let mut ranges = Vec::<[usize; 2]>::new();
    if let Some(range @ [start, end]) = request.target_logical_scalar_range {
        if start > end || end > scalar_len {
            return Err(WellfriendError::invalid_input(
                "universal text target logical scalar range is outside the page model",
            ));
        }
        let selected = model
            .logical_text
            .chars()
            .skip(start)
            .take(end.saturating_sub(start))
            .collect::<String>();
        if selected != request.source_text {
            return Err(WellfriendError::invalid_input(
                "universal text target logical scalar range is stale for source_text",
            ));
        }
        ranges.push(range);
    } else if request.source_text.is_empty() {
        return Ok(Vec::new());
    } else {
        for (byte_start, matched) in model.logical_text.match_indices(&request.source_text) {
            if ranges.len() >= MAX_TEXT_CANDIDATES {
                return Err(WellfriendError::ResourceLimit(
                    "universal text candidate count exceeds the governed limit 1000000"
                        .to_string(),
                ));
            }
            let byte_end = byte_start.saturating_add(matched.len());
            ranges.push([
                model.logical_text[..byte_start].chars().count(),
                model.logical_text[..byte_end].chars().count(),
            ]);
        }
    }

    let mut output = Vec::new();
    for [start, end] in ranges {
        let spans = model
            .source_spans
            .iter()
            .filter(|span| {
                if start == end {
                    false
                } else {
                    span.logical_range[0] < end && span.logical_range[1] > start
                }
            })
            .cloned()
            .collect::<Vec<_>>();
        let provenance_covered = if start == end {
            model.source_spans.iter().any(|span| {
                span.logical_range[0] == start || span.logical_range[1] == start
            })
        } else {
            let mut covered_until = start;
            for span in &spans {
                let overlap_start = span.logical_range[0].max(start);
                let overlap_end = span.logical_range[1].min(end);
                if overlap_start > covered_until || overlap_end <= overlap_start {
                    break;
                }
                covered_until = covered_until.max(overlap_end);
            }
            covered_until == end
        };
        let exact = provenance_covered;

        if spans.len() == 1
            && existing.iter().any(|candidate| {
                candidate.kind == "text_source_instruction"
                    && candidate.source_identity["stream_object"].as_u64()
                        == Some(u64::from(spans[0].stream_object))
                    && candidate.source_identity["stream_generation"].as_u64()
                        == Some(u64::from(spans[0].stream_generation))
                    && candidate.source_identity["decoded_byte_range"]
                        == json!(spans[0].byte_range)
            })
        {
            continue;
        }

        let page_bytes = request.page.to_le_bytes();
        let start_bytes = start.to_le_bytes();
        let end_bytes = end.to_le_bytes();
        let span_identity = serde_json::to_vec(&spans).map_err(json_error)?;
        let candidate_id = stable_id(
            "text-range-v2",
            &[
                revision.as_bytes(),
                &page_bytes,
                &start_bytes,
                &end_bytes,
                span_identity.as_slice(),
            ],
        );
        if request
            .source_instruction_id
            .as_deref()
            .is_some_and(|selected| selected != candidate_id)
        {
            continue;
        }
        output.push(UniversalCandidateV2 {
            candidate_id,
            page: request.page,
            kind: "multi_run_text_range".to_string(),
            source_identity: json!({
                "revision_id": revision,
                "page": request.page,
                "logical_scalar_range": [start, end],
                "source_spans": spans,
                "selection_model": "page_logical_unicode_scalar_range",
            }),
            confidence: if exact { 1.0 } else { 0.50 },
            exact,
            shared_resource: false,
            approval_reason: Some(if exact {
                "provenance-complete logical text range requires an exact candidate decision; boundary residuals and all touched streams are committed atomically"
                    .to_string()
            } else {
                "match contains a gap without source provenance and cannot be rewritten without inventing ownership"
                    .to_string()
            }),
        });
    }
    Ok(output)
}

fn bind_text_candidate_to_request(
    request: &mut SceneTextEditRequest,
    candidate: &UniversalCandidateV2,
) -> Result<()> {
    if !candidate.exact {
        return Err(WellfriendError::invalid_input(
            "universal text candidate is not exact and cannot be applied",
        ));
    }
    match candidate.kind.as_str() {
        "text_source_instruction" => {
            request.source_instruction_id = Some(candidate.candidate_id.clone());
            request.target_logical_scalar_range = None;
        }
        "multi_run_text_range" => {
            let range = candidate.source_identity["logical_scalar_range"]
                .as_array()
                .filter(|range| range.len() == 2)
                .and_then(|range| {
                    Some([
                        usize::try_from(range[0].as_u64()?).ok()?,
                        usize::try_from(range[1].as_u64()?).ok()?,
                    ])
                })
                .ok_or_else(|| {
                    WellfriendError::MalformedPdf(
                        "universal text range candidate has no valid logical scalar range"
                            .to_string(),
                    )
                })?;
            request.source_instruction_id = None;
            request.target_logical_scalar_range = Some(range);
        }
        _ => {
            return Err(WellfriendError::invalid_input(
                "universal text candidate has an incompatible candidate kind",
            ));
        }
    }
    Ok(())
}

fn selected_source_text_span(
    input: &[u8],
    request: &SceneTextEditRequest,
) -> Option<crate::advanced_editing::MultiRunSourceSpan> {
    let model = analyze_multi_run_text_range(input, request.page).ok()?;
    if let Some([start, end]) = request.target_logical_scalar_range {
        return model.source_spans.into_iter().find(|span| {
            if start == end {
                span.logical_range[0] == start || span.logical_range[1] == start
            } else {
                span.logical_range[0] < end && span.logical_range[1] > start
            }
        });
    }
    let selected_identity = request.source_instruction_id.as_deref().and_then(|selected| {
        crate::source_editing::operator_text_provenance(
            input,
            request.page,
            &request.source_text,
            &request.replacement_text,
        )
        .ok()?
        .source_instructions
        .into_iter()
        .find(|identity| identity.instruction_id == selected)
    });
    model.source_spans.into_iter().find(|span| {
        selected_identity.as_ref().map_or_else(
            || request.source_text.is_empty() || request.source_text.contains(span.text.as_str()),
            |identity| {
                span.stream_object == identity.stream_object
                    && span.stream_generation == identity.stream_generation
                    && span.byte_range == identity.decoded_byte_range
            },
        )
    })
}

fn source_font_family(input: &[u8], request: &SceneTextEditRequest) -> Option<String> {
    let source = selected_source_text_span(input, request)?;
    let engine = ContentEngine::open_bytes(input.to_vec()).ok()?;
    let resources = engine.get_page_resources(request.page).ok()?;
    let dictionary = resources.fonts.get(&source.font_resource)?;
    dictionary
        .get("BaseFont")
        .and_then(PdfObject::as_name)
        .map(str::to_string)
        .or_else(|| Some(source.font_resource.clone()))
}

fn source_font_program(input: &[u8], request: &SceneTextEditRequest) -> Option<Vec<u8>> {
    let source = selected_source_text_span(input, request)?;
    let engine = ContentEngine::open_bytes(input.to_vec()).ok()?;
    let resources = engine.get_page_resources(request.page).ok()?;
    let reader = engine.document().reader();
    let mut font = PdfObject::Dictionary(resources.fonts.get(&source.font_resource)?.clone());
    if font
        .as_dict()
        .and_then(|dict| dict.get("Subtype"))
        .and_then(PdfObject::as_name)
        == Some("Type0")
    {
        let descendant = font
            .as_dict()?
            .get("DescendantFonts")?
            .as_array()?
            .first()?
            .clone();
        font = reader.resolve(descendant).ok()?;
    }
    let descriptor = font.as_dict()?.get("FontDescriptor")?.clone();
    let descriptor = reader.resolve(descriptor).ok()?;
    let descriptor = descriptor.as_dict()?;
    let program = ["FontFile2", "FontFile3", "FontFile"]
        .iter()
        .find_map(|key| descriptor.get(key).cloned())?;
    let program = reader.resolve(program).ok()?;
    let decoded = decode_stream_lossless_with_limits(
        &program,
        reader,
        &DecodeLimits {
            max_decoded_bytes_per_stream: 128 * 1024 * 1024,
        },
    )
    .ok()?;
    (decoded.status == StreamDecodeStatus::Complete).then_some(decoded.data)
}

fn universal_substitution_report_v2(
    requested_family: &str,
    text: &str,
    policy: Option<&str>,
    source_font_bytes: Option<&[u8]>,
    approved_asset: Option<&crate::editing_transactions::ApprovedFontAsset>,
) -> Value {
    let mut report = substitution_report_with_source_font(
        requested_family,
        text,
        policy,
        source_font_bytes,
    );
    let Some(asset) = approved_asset else {
        return report;
    };
    let valid_name = !asset.lookup_name.trim().is_empty() && asset.lookup_name.len() <= 255;
    let bounded = !asset.bytes.is_empty() && asset.bytes.len() <= 256 * 1024 * 1024;
    let parsed = bounded
        .then(|| ttf_parser::Face::parse(&asset.bytes, 0).ok())
        .flatten();
    let missing_scalars = parsed
        .as_ref()
        .map(|face| {
            text.chars()
                .filter(|character| !character.is_control() && face.glyph_index(*character).is_none())
                .map(|character| format!("U+{:04X}", character as u32))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let eligible = valid_name && bounded && parsed.is_some() && missing_scalars.is_empty();
    let candidate = json!({
        "family_name": asset.lookup_name,
        "lookup_name": asset.lookup_name,
        "source": "caller_governed_plan_bound_font_asset",
        "font_sha256": digest_hex(&asset.bytes),
        "byte_length": asset.bytes.len(),
        "missing_scalars": missing_scalars,
        "eligible_for_approval": eligible,
        "embedding_policy": "caller_governed_asset_explicitly_bound_to_plan",
    });
    if let Some(candidates) = report
        .get_mut("ranked_candidates")
        .and_then(Value::as_array_mut)
    {
        candidates.insert(0, candidate);
    }
    if eligible {
        if let Some(approved) = report
            .get_mut("approved_candidates")
            .and_then(Value::as_array_mut)
        {
            approved.retain(|value| value.as_str() != Some(asset.lookup_name.as_str()));
            approved.insert(0, Value::String(asset.lookup_name.clone()));
        }
        report["status"] = Value::String("caller_font_asset_eligible_for_approval".to_string());
        report["chosen_substitute"] = Value::String(asset.lookup_name.clone());
    } else {
        report["caller_font_asset_error"] = Value::String(
            if !valid_name {
                "lookup name is empty or exceeds 255 bytes"
            } else if !bounded {
                "font program is empty or exceeds 256 MiB"
            } else if parsed.is_none() {
                "font program is not a supported sfnt/OpenType face"
            } else {
                "font program does not cover every requested Unicode scalar"
            }
            .to_string(),
        );
    }
    report
}

fn universal_input_recovery_state(input: &[u8]) -> Value {
    match crate::reader::PdfReader::from_bytes_strict(input.to_vec()) {
        Ok(_) => json!({
            "strict_open": true,
            "deterministic_repair_required": false,
            "classification": "iso_structurally_valid_for_engine",
        }),
        Err(error) => json!({
            "strict_open": false,
            "permissive_open": true,
            "deterministic_repair_required": true,
            "classification": "deterministically_recoverable_pdf",
            "strict_error": error.to_string(),
            "repair_is_authorized_full_rewrite_only": true,
        }),
    }
}

fn plan_id(
    revision_id: &str,
    requested: &UniversalEditOperationV2,
    execution: &UniversalEditOperationV2,
    policy: &UniversalEditPolicyV2,
) -> Result<String> {
    let requested = serde_json::to_vec(requested).map_err(json_error)?;
    let execution = serde_json::to_vec(execution).map_err(json_error)?;
    let policy = serde_json::to_vec(policy).map_err(json_error)?;
    Ok(stable_id(
        "edit-plan-v2",
        &[
            revision_id.as_bytes(),
            requested.as_slice(),
            execution.as_slice(),
            policy.as_slice(),
        ],
    ))
}

fn approval_digest(
    plan: &UniversalEditPlanV2,
    decision: &UniversalApprovalDecisionV2,
) -> Result<String> {
    let decision = serde_json::to_vec(decision).map_err(json_error)?;
    Ok(stable_id(
        "approval-v2",
        &[
            plan.plan_id.as_bytes(),
            plan.revision_id.as_bytes(),
            decision.as_slice(),
        ],
    ))
}

fn validate_approval(plan: &UniversalEditPlanV2, token: &UniversalApprovalTokenV2) -> Result<()> {
    if token.plan_id != plan.plan_id || token.revision_id != plan.revision_id {
        return Err(WellfriendError::invalid_input(
            "universal editing approval token is stale or belongs to another plan",
        ));
    }
    let canonical = create_universal_approval_token_v2(plan, token.decision.clone())?;
    if canonical.binding_digest != token.binding_digest {
        return Err(WellfriendError::AuthenticationFailure(
            "universal editing approval token digest mismatch".to_string(),
        ));
    }
    Ok(())
}

fn enforce_universal_signature_policy(
    policy: &EditPolicyReport,
    mode: UniversalMutationModeV2,
) -> Result<()> {
    if mode == UniversalMutationModeV2::PreserveSignatures
        && matches!(
            policy.decision,
            EditPolicyDecision::BlockedBySignaturePolicy
                | EditPolicyDecision::ExplicitOverrideRequired
                | EditPolicyDecision::FullRewriteRequired
        )
    {
        return Err(WellfriendError::UnsupportedFeature(
            "universal editing preserve_signatures mode is denied by the current signature policy"
                .to_string(),
        ));
    }
    Ok(())
}

fn no_change_result(plan: &UniversalEditPlanV2, revision_id: &str) -> UniversalEditResultV2 {
    UniversalEditResultV2 {
        schema_version: UNIVERSAL_EDITING_SCHEMA_VERSION.to_string(),
        plan_id: plan.plan_id.clone(),
        transaction_id: String::new(),
        outcome: match plan.state {
            UniversalPlanStateV2::PolicyDenied => UniversalEditOutcomeV2::PolicyDenied,
            UniversalPlanStateV2::TargetNotFound => UniversalEditOutcomeV2::TargetNotFound,
            UniversalPlanStateV2::IrrecoverableInput => UniversalEditOutcomeV2::IrrecoverableInput,
            _ => UniversalEditOutcomeV2::ApprovalRequired,
        },
        changed: false,
        input_revision_id: revision_id.to_string(),
        output_revision_id: revision_id.to_string(),
        affected_pages: Vec::new(),
        affected_objects: Vec::new(),
        cloned_resources: Vec::new(),
        operation_report: Value::Null,
        render_invalidation: json!({"required": false, "reason": "no_change"}),
        signature_impact: plan.signature_impact.clone(),
        conformance_impact: plan.conformance_impact.clone(),
        inverse: json!({"required": false, "reason": "no_change"}),
        issues: plan
            .approval_reasons
            .iter()
            .map(|message| json!({"message": message, "no_change_proof": true}))
            .collect(),
    }
}

fn stable_id(kind: &str, values: &[&[u8]]) -> String {
    let mut digest = Sha256::new();
    digest.update(kind.as_bytes());
    digest.update([0]);
    for value in values {
        digest.update(value);
        digest.update([0]);
    }
    let encoded = format!("{:x}", digest.finalize());
    format!("{kind}-{}", &encoded[..24])
}

pub(crate) fn digest_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn digest_hex_cancellable(bytes: &[u8], context: &str) -> Result<String> {
    const CHUNK_BYTES: usize = 1024 * 1024;
    let mut digest = Sha256::new();
    for chunk in bytes.chunks(CHUNK_BYTES) {
        crate::cancel::check_current_cancel(context)?;
        digest.update(chunk);
    }
    Ok(format!("{:x}", digest.finalize()))
}

pub(crate) fn revision_id(bytes: &[u8]) -> String {
    stable_id("revision", &[bytes, &bytes.len().to_le_bytes()])
}

fn json_error(error: serde_json::Error) -> WellfriendError {
    WellfriendError::invalid_input(format!("universal editing JSON error: {error}"))
}
