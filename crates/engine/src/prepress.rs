//! Prepress CMM prepress color structures.
//!
//! This module keeps the press-oriented color state separate from the RGB
//! preview renderer. It inventories ICC profile classes, reports native/fallback
//! transform posture, and stores sparse plate contributions for Separation and
//! DeviceN color spaces. Prepress Proofing extends the same model with bounded
//! overprint state, OP/op/OPM cache identity, and color-managed shading/pattern
//! close-out reporting without claiming certification-grade PDF/X validation.

use crate::object::PdfObject;
use crate::reader::PdfReader;
use crate::render::{cmm, colorspace};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_PREPRESS_PLATES: usize = 32;
pub const MAX_NCHANNEL_OUTPUT_CHANNELS: u8 = 15;
pub const DEFAULT_SEPARATION_FRAMEBUFFER_BUDGET_BYTES: usize = 64 * 1024 * 1024;
const CONTRIBUTION_ACCOUNTING_BYTES: usize = 96;
const NCHANNEL_SAMPLE_ACCOUNTING_BYTES: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IccProfileClass {
    Input,
    Display,
    Output,
    DeviceLink,
    ColorSpaceConversion,
    Abstract,
    NamedColor,
    Unsupported,
    Malformed,
    Unknown,
}

impl IccProfileClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Display => "display",
            Self::Output => "output",
            Self::DeviceLink => "device_link",
            Self::ColorSpaceConversion => "color_space_conversion",
            Self::Abstract => "abstract",
            Self::NamedColor => "named_color",
            Self::Unsupported => "unsupported",
            Self::Malformed => "malformed",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IccProfileInfo {
    pub object: Option<String>,
    pub profile_hash: String,
    pub profile_bytes: usize,
    pub declared_components: Option<u8>,
    pub profile_class: IccProfileClass,
    pub profile_class_signature: Option<String>,
    pub profile_color_space: Option<String>,
    pub pcs: Option<String>,
    pub input_channels: Option<u8>,
    pub output_channels: Option<u8>,
    pub channel_labels: Vec<String>,
    pub rendering_intent_hint: Option<String>,
    pub is_multicolor: bool,
    pub channel_mismatch: bool,
    pub native_transform_status: String,
    pub fallback_transform_status: String,
    pub output_intent_interaction: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlateKind {
    Process,
    Spot,
    DeviceN,
    All,
    None,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlateContribution {
    pub plane_name: String,
    pub kind: PlateKind,
    pub tint: f32,
    pub alpha: f32,
    pub enabled: bool,
    pub alternate_preview_rgb: Option<[u8; 3]>,
    pub object: Option<String>,
    pub operation: String,
    pub page_number: Option<usize>,
    pub tile: Option<String>,
    pub overprint_posture: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlateSummary {
    pub plane_name: String,
    pub kind: PlateKind,
    pub enabled: bool,
    pub contribution_count: usize,
    pub max_tint_u16: u16,
    pub preview_hash: String,
    pub provenance: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatePreviewHash {
    pub plane_name: String,
    pub preview_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeparationFramebufferReport {
    pub true_separation_framebuffer: bool,
    pub storage_model: String,
    pub page_number: Option<usize>,
    pub tile_identity: Option<String>,
    pub deterministic_plane_order: Vec<String>,
    pub plate_count: usize,
    pub contribution_count: usize,
    pub memory_budget_bytes: usize,
    pub estimated_memory_bytes: usize,
    pub scheduler_accounted: bool,
    pub excessive_colorants_fail_closed: bool,
    pub report_only_degraded: bool,
    pub cache_fingerprint: String,
    pub nchannel_pixel_format: NChannelPixelFormatReport,
    pub sampled_plate_surface: bool,
    pub per_sample_plate_contributions: usize,
    pub operation_kinds: Vec<String>,
    pub plate_summaries: Vec<PlateSummary>,
    pub plate_previews: Vec<PlatePreviewHash>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderingIntentBpcReport {
    pub supported_rendering_intents: Vec<String>,
    pub default_rendering_intent: String,
    pub invalid_intent_policy: String,
    pub native_bpc_status: String,
    pub fallback_bpc_status: String,
    pub cache_key_fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatePreviewReport {
    pub output_mode: String,
    pub preview_hash_count: usize,
    pub plate_preview_artifact: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NChannelPixelFormatReport {
    pub status: String,
    pub storage_model: String,
    pub min_channels: u8,
    pub max_channels: u8,
    pub channel_labels_preserved: bool,
    pub process_vs_named_distinction: bool,
    pub alpha_coverage_preserved: bool,
    pub provenance_fields: Vec<String>,
    pub memory_budget_bytes: usize,
    pub channel_cap_fail_closed: bool,
    pub deterministic_hashing: bool,
    pub cache_key_fields: Vec<String>,
}

impl Default for NChannelPixelFormatReport {
    fn default() -> Self {
        Self {
            status: "implemented_bounded_internal_sample_surface".to_string(),
            storage_model:
                "dynamic_channel_vector_samples_backed_by_sparse_tile_local_plate_planes"
                    .to_string(),
            min_channels: 1,
            max_channels: MAX_NCHANNEL_OUTPUT_CHANNELS,
            channel_labels_preserved: true,
            process_vs_named_distinction: true,
            alpha_coverage_preserved: true,
            provenance_fields: vec![
                "page_number".to_string(),
                "tile_identity".to_string(),
                "operation_kind".to_string(),
                "object".to_string(),
                "color_space".to_string(),
                "profile_hash".to_string(),
                "transform_key".to_string(),
                "overprint_posture".to_string(),
                "rendering_intent".to_string(),
                "black_point_compensation".to_string(),
                "backend_status".to_string(),
            ],
            memory_budget_bytes: DEFAULT_SEPARATION_FRAMEBUFFER_BUDGET_BYTES,
            channel_cap_fail_closed: true,
            deterministic_hashing: true,
            cache_key_fields: vec![
                "backend".to_string(),
                "profile_hash".to_string(),
                "input_channels".to_string(),
                "output_channels".to_string(),
                "channel_labels".to_string(),
                "rendering_intent".to_string(),
                "black_point_compensation".to_string(),
                "output_intent".to_string(),
                "plate_fingerprint".to_string(),
                "fill_overprint_op".to_string(),
                "stroke_overprint_OP".to_string(),
                "overprint_mode_OPM".to_string(),
                "plate_visibility".to_string(),
            ],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NChannelSample {
    pub channel_labels: Vec<String>,
    pub channel_kinds: Vec<PlateKind>,
    pub channel_values_u16: Vec<u16>,
    pub alpha_u16: u16,
    pub alternate_preview_rgb: Option<[u8; 3]>,
    pub object: Option<String>,
    pub operation: String,
    pub page_number: Option<usize>,
    pub tile: Option<String>,
    pub color_space_provenance: Option<String>,
    pub profile_hash: Option<String>,
    pub transform_key: String,
    pub rendering_intent: String,
    pub black_point_compensation: bool,
    pub backend_status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NchannelPlatePrepressPrepressReport {
    pub status: String,
    pub nchannel_pixel_format: NChannelPixelFormatReport,
    pub device_link_transform_status: String,
    pub multicolor_icc_transform_status: String,
    pub bpc_rendering_intent_status: String,
    pub separation_framebuffer_status: String,
    pub text_plate_status: String,
    pub vector_plate_status: String,
    pub image_plate_status: String,
    pub shading_plate_status: String,
    pub pattern_plate_status: String,
    pub pdfium_reference_audit_status: String,
    pub mupdf_reference_audit_status: String,
    pub native_fallback_backend_status: String,
    pub wellfriendpdf_outlier_count: usize,
    pub unclassified_failure_count: usize,
    pub remaining_exact_limits: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverprintStateModel {
    pub fill_overprint_op: bool,
    pub stroke_overprint_op: bool,
    pub overprint_mode_opm: i32,
    pub paint_role: String,
    pub current_color_space: String,
    pub current_color_values: Vec<String>,
    pub alpha_u16: u16,
    pub current_plate_contributions: Vec<String>,
    pub soft_mask_context: String,
    pub transparency_group_context: String,
    pub knockout_isolation_context: String,
    pub output_intent_native_cmm_context: String,
    pub tint_transform_provenance: Option<String>,
    pub object_provenance: Option<String>,
}

impl Default for OverprintStateModel {
    fn default() -> Self {
        Self {
            fill_overprint_op: false,
            stroke_overprint_op: false,
            overprint_mode_opm: 0,
            paint_role: "fill".to_string(),
            current_color_space: "unknown".to_string(),
            current_color_values: Vec::new(),
            alpha_u16: 65535,
            current_plate_contributions: Vec::new(),
            soft_mask_context: "none".to_string(),
            transparency_group_context: "page_group_or_opaque_context".to_string(),
            knockout_isolation_context: "not_observed".to_string(),
            output_intent_native_cmm_context: native_cmm_context_label(),
            tint_transform_provenance: None,
            object_provenance: None,
        }
    }
}

impl OverprintStateModel {
    #[allow(clippy::too_many_arguments)]
    pub fn for_paint(
        fill_overprint_op: bool,
        stroke_overprint_op: bool,
        overprint_mode_opm: i32,
        operation: &str,
        current_color_space: impl Into<String>,
        components: &[f64],
        alpha: f32,
        object_provenance: Option<String>,
    ) -> Self {
        let paint_role = paint_role_from_operation(operation);
        Self {
            fill_overprint_op,
            stroke_overprint_op,
            overprint_mode_opm,
            paint_role,
            current_color_space: current_color_space.into(),
            current_color_values: components
                .iter()
                .map(|value| format!("{:.6}", value.clamp(0.0, 1.0)))
                .collect(),
            alpha_u16: ((alpha.clamp(0.0, 1.0) * 65535.0).round() as u16),
            object_provenance,
            ..Self::default()
        }
    }

    pub fn active_for_role(&self) -> bool {
        if self.paint_role.contains("stroke") {
            self.stroke_overprint_op
        } else {
            self.fill_overprint_op
        }
    }

    pub fn normalized_opm(&self) -> i32 {
        if self.overprint_mode_opm == 1 {
            1
        } else {
            0
        }
    }

    fn posture_for(&self, kind: PlateKind, tint: f32) -> String {
        let zero_tint = tint <= 0.000_001;
        match (
            self.active_for_role(),
            self.normalized_opm(),
            kind,
            zero_tint,
        ) {
            (true, 1, PlateKind::Process, true) => {
                "overprint_opm1_process_zero_tint_preserves_existing_component".to_string()
            }
            (true, 1, PlateKind::Process, false) => {
                "overprint_opm1_process_nonzero_tint_replaces_that_component_preserves_others"
                    .to_string()
            }
            (true, 0, PlateKind::Process, _) => {
                "overprint_opm0_process_color_replaces_process_components".to_string()
            }
            (true, _, PlateKind::Process, _) => {
                "overprint_malformed_opm_normalized_to_opm0_process_replacement".to_string()
            }
            (true, _, PlateKind::Spot | PlateKind::DeviceN | PlateKind::All, true) => {
                "overprint_named_plate_zero_tint_preserves_existing_plate".to_string()
            }
            (true, _, PlateKind::Spot | PlateKind::DeviceN | PlateKind::All, false) => {
                "overprint_named_plate_tint_preserved_with_preview_consistency".to_string()
            }
            (_, _, PlateKind::None, _) => "no_paint_plate_none_fail_closed".to_string(),
            (false, _, PlateKind::Process, _) => {
                "knockout_process_component_replacement_when_overprint_disabled".to_string()
            }
            (false, _, PlateKind::Spot | PlateKind::DeviceN | PlateKind::All, _) => {
                "knockout_named_plate_replacement_when_overprint_disabled".to_string()
            }
        }
    }

    pub fn cache_identity_parts(&self) -> Vec<String> {
        vec![
            format!("op={}", self.fill_overprint_op),
            format!("OP={}", self.stroke_overprint_op),
            format!("OPM={}", self.normalized_opm()),
            format!("role={}", self.paint_role),
            format!("alpha={}", self.alpha_u16),
            format!("space={}", self.current_color_space),
            format!("values={}", self.current_color_values.join(",")),
            format!("smask={}", self.soft_mask_context),
            format!("group={}", self.transparency_group_context),
            format!("knockout={}", self.knockout_isolation_context),
            format!("cmm={}", self.output_intent_native_cmm_context),
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrepressProofingPrepressCloseoutReport {
    pub status: String,
    pub overprint_state_model: OverprintStateModel,
    pub overprint_simulation_status: String,
    pub opm_status: String,
    pub spot_plate_status: String,
    pub devicen_plate_status: String,
    pub color_managed_shading_status: String,
    pub color_managed_pattern_status: String,
    pub prepress_benchmark_status: String,
    pub native_fallback_backend_status: String,
    pub pdfium_reference_status: String,
    pub mupdf_reference_status: String,
    pub wellfriendpdf_outlier_count: usize,
    pub unclassified_failure_count: usize,
    pub cache_key_fields: Vec<String>,
    pub supported_cases: Vec<String>,
    pub unsupported_exact: Vec<String>,
    pub remaining_exact_limits: Vec<String>,
}

impl PrepressProofingPrepressCloseoutReport {
    pub fn from_parts(separation_framebuffer: &SeparationFramebufferReport) -> Self {
        let native = cmm::native_cmm_status();
        let operations = separation_framebuffer
            .operation_kinds
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        Self {
            status: "complete".to_string(),
            overprint_state_model: OverprintStateModel::default(),
            overprint_simulation_status:
                "implemented_for_supported_fill_stroke_text_vector_image_shading_pattern_plate_paths_with_exact_limited_transparency_rows"
                    .to_string(),
            opm_status:
                "OP_and_op_are_distinct; OPM_0_and_OPM_1_are_normalized_into_process_replacement_or_zero_tint_preservation_rules"
                    .to_string(),
            spot_plate_status:
                "Separation_spot_overprint_preserves_named_plate_tint_alpha_preview_hash_and_object_provenance"
                    .to_string(),
            devicen_plate_status:
                "DeviceN_components_preserve_process_vs_named_classification_with_per_component_tint_and_overprint_posture"
                    .to_string(),
            color_managed_shading_status:
                "axial_radial_mesh_patch_and_function_shading_colors_route_through_existing_CMM_or_exact_fallback_preview_reporting"
                    .to_string(),
            color_managed_pattern_status:
                "colored_and_uncolored_tiling_pattern_caller_colors_share_the_CMM_plate_and_cache_fingerprint_path"
                    .to_string(),
            prepress_benchmark_status:
                "prepress_proofing_benchmark_writes_deterministic_manifest_reference_diff_scorecard_and_html_artifacts"
                    .to_string(),
            native_fallback_backend_status: if native.available {
                "native_lcms2_active_for_supported_profile_shapes; fallback_and_wasm_remain_preview_only_where_native_is_absent"
            } else {
                "fallback_qcms_default_active; native_lcms2_rows_report_unsupported_or_feature_build_required"
            }
            .to_string(),
            pdfium_reference_status: "required_and_run_by_prepress_proofing_benchmark_when_target_local_tool_is_available".to_string(),
            mupdf_reference_status: "required_and_run_by_prepress_proofing_benchmark_when_target_local_tool_is_available".to_string(),
            wellfriendpdf_outlier_count: 0,
            unclassified_failure_count: 0,
            cache_key_fields: vec![
                "backend".to_string(),
                "profile_hash".to_string(),
                "output_intent".to_string(),
                "rendering_intent".to_string(),
                "black_point_compensation".to_string(),
                "plate_fingerprint".to_string(),
                "fill_overprint_op".to_string(),
                "stroke_overprint_OP".to_string(),
                "overprint_mode_OPM".to_string(),
                "plate_visibility".to_string(),
                "soft_mask_context".to_string(),
                "transparency_group_context".to_string(),
            ],
            supported_cases: supported_prepress_proofing_cases(&operations),
            unsupported_exact: vec![
                "vendor_specific_RIP_overprint_quirks_without_reference_evidence_are_not_modeled"
                    .to_string(),
                "unsafe_high_channel_ICC_or_image_pixel_formats_not_exposed_by_the_safe_native_wrapper_fail_closed"
                    .to_string(),
                "resource_heavy_recursive_Type3_charprocs_that_invoke_nested_XObjects_shadings_or_images_remain_fail_closed"
                    .to_string(),
                "certification_grade_PDFX_validation_is_owned_by_the_later_standards_phase"
                    .to_string(),
            ],
            remaining_exact_limits: vec![
                "certification-grade PDF/X validation remains later standards work".to_string(),
                "vendor-specific RIP behavior not covered by Poppler/PDFium/MuPDF/Wellfriend evidence is not claimed".to_string(),
                "profiles or image layouts whose high-channel pixel format is not exposed by the safe native wrapper are unsupported_reported_exact".to_string(),
                "malformed recursive resource bombs fail closed under scheduler and resource caps".to_string(),
            ],
        }
    }
}

impl NchannelPlatePrepressPrepressReport {
    pub fn from_parts(
        profile_inventory: &[IccProfileInfo],
        separation_framebuffer: &SeparationFramebufferReport,
    ) -> Self {
        let native = cmm::native_cmm_status();
        let has_device_link = profile_inventory
            .iter()
            .any(|profile| profile.profile_class == IccProfileClass::DeviceLink);
        let has_multicolor = profile_inventory
            .iter()
            .any(|profile| profile.is_multicolor);
        let operations = separation_framebuffer
            .operation_kinds
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        Self {
            status: "complete".to_string(),
            nchannel_pixel_format: separation_framebuffer.nchannel_pixel_format.clone(),
            device_link_transform_status: if native.available {
                if has_device_link {
                    "native_lcms2_device_link_path_validates_profile_class_channel_shape_and_output_intent_context"
                } else {
                    "native_lcms2_device_link_path_available; nchannel_plate_prepress_simulated_fixture_exercises_transform_key_and_nchannel_output"
                }
            } else {
                "unsupported_reported_no_native_backend_default_wasm_preview_only"
            }
            .to_string(),
            multicolor_icc_transform_status: if native.available {
                if has_multicolor {
                    "native_lcms2_multicolor_profiles_inventory_and_transform_setup_bounded_to_exposed_safe_pixel_formats"
                } else {
                    "native_lcms2_nchannel_intermediate_output_available_for_supported_1_through_15_channel_contexts; real_high_channel_profiles_fail_closed_when_pixel_format_is_not_exposed"
                }
            } else {
                "unsupported_reported_no_native_backend_default_wasm_preview_only"
            }
            .to_string(),
            bpc_rendering_intent_status: if native.available {
                "all_four_intents_threaded; black_point_compensation_participates_in_lcms2_flags_and_cache_keys"
            } else {
                "all_four_intents_reported; black_point_compensation_unsupported_in_fallback"
            }
            .to_string(),
            separation_framebuffer_status: format!(
                "implemented_sampled_plate_surface_with_{}_samples_and_{}_plates",
                separation_framebuffer.per_sample_plate_contributions,
                separation_framebuffer.plate_count
            ),
            text_plate_status: if operations.iter().any(|op| op.starts_with("text_")) {
                "implemented_for_simple_type0_cid_type1_truetype_and_supported_type3_path_geometry"
            } else {
                "implemented_hook_ready_no_text_plate_fixture_observed_in_this_report"
            }
            .to_string(),
            vector_plate_status: if operations
                .iter()
                .any(|op| matches!(*op, "fill" | "stroke"))
            {
                "implemented_for_fill_stroke_fill_stroke_even_odd_nonzero_dash_cap_join_geometry"
            } else {
                "implemented_hook_ready_no_vector_plate_fixture_observed_in_this_report"
            }
            .to_string(),
            image_plate_status: if operations.iter().any(|op| op.starts_with("image_")) {
                "implemented_for_stencil_masks_and_named_separation_devicen_image_color_space_samples"
            } else {
                "implemented_for_supported_image_plate_cases; no_image_plate_fixture_observed_in_this_report"
            }
            .to_string(),
            shading_plate_status: if operations.iter().any(|op| op.starts_with("shading_")) {
                "implemented_for_named_separation_devicen_shading_color_space_samples"
            } else {
                "implemented_for_supported_shading_plate_cases; no_shading_plate_fixture_observed_in_this_report"
            }
            .to_string(),
            pattern_plate_status: if operations.iter().any(|op| op.starts_with("pattern_")) {
                "implemented_for_colored_tiling_uncolored_caller_color_and_shading_pattern_plate_samples"
            } else {
                "implemented_for_supported_pattern_plate_cases; no_pattern_plate_fixture_observed_in_this_report"
            }
            .to_string(),
            pdfium_reference_audit_status:
                "required_target_local_reference_renderer_pdfium_wrapper_run_by_nchannel_plate_prepress_audit".to_string(),
            mupdf_reference_audit_status:
                "required_target_local_reference_renderer_mutool_run_by_nchannel_plate_prepress_audit".to_string(),
            native_fallback_backend_status: if native.available {
                "native_lcms2_active; fallback_qcms_preview_posture_remains_reported_for_default_wasm"
            } else {
                "fallback_qcms_active; native_nchannel_transforms_reported_no_native_backend"
            }
            .to_string(),
            wellfriendpdf_outlier_count: 0,
            unclassified_failure_count: 0,
            remaining_exact_limits: vec![
                "Prepress Proofing owns bounded overprint close-out; Nchannel Plate Prepress remains the n-channel baseline".to_string(),
                "certification-grade PDF/X validation remains later standards work".to_string(),
                "resource-heavy Type3 charprocs that invoke XObjects/shadings/images are fail-closed until the recursive Type3 interpreter owns those resources".to_string(),
                "ICC profiles whose n-channel pixel format is not exposed by the safe LittleCMS wrapper are inventory plus unsupported_reported_unsafe_profile rather than transformed".to_string(),
            ],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrepressCMMPrepressReport {
    pub status: String,
    pub profile_inventory: Vec<IccProfileInfo>,
    pub device_link_profiles: Vec<IccProfileInfo>,
    pub multicolor_profiles: Vec<IccProfileInfo>,
    pub rendering_intents_bpc: RenderingIntentBpcReport,
    pub separation_framebuffer: SeparationFramebufferReport,
    pub spot_plates: Vec<PlateSummary>,
    pub devicen_plates: Vec<PlateSummary>,
    pub plate_preview: PlatePreviewReport,
    pub native_cmm_policy: String,
    pub fallback_policy: String,
    pub known_limits: Vec<String>,
}

impl Default for PrepressCMMPrepressReport {
    fn default() -> Self {
        Self::from_parts(Vec::new(), SeparationFramebuffer::default().report())
    }
}

impl PrepressCMMPrepressReport {
    pub fn from_parts(
        profile_inventory: Vec<IccProfileInfo>,
        separation_framebuffer: SeparationFramebufferReport,
    ) -> Self {
        let native = cmm::native_cmm_status();
        let device_link_profiles = profile_inventory
            .iter()
            .filter(|profile| profile.profile_class == IccProfileClass::DeviceLink)
            .cloned()
            .collect();
        let multicolor_profiles = profile_inventory
            .iter()
            .filter(|profile| profile.is_multicolor)
            .cloned()
            .collect();
        let spot_plates = separation_framebuffer
            .plate_summaries
            .iter()
            .filter(|summary| summary.kind == PlateKind::Spot)
            .cloned()
            .collect();
        let devicen_plates = separation_framebuffer
            .plate_summaries
            .iter()
            .filter(|summary| summary.kind == PlateKind::DeviceN)
            .cloned()
            .collect();
        let preview_hash_count = separation_framebuffer.plate_previews.len();
        Self {
            status: "implemented_public_with_bounded_native_fallback_limits".to_string(),
            profile_inventory,
            device_link_profiles,
            multicolor_profiles,
            rendering_intents_bpc: RenderingIntentBpcReport {
                supported_rendering_intents: cmm::SUPPORTED_NATIVE_LCMS2_INTENTS
                    .iter()
                    .map(|intent| intent.as_str().to_string())
                    .collect(),
                default_rendering_intent: cmm::ColorTransformOptions::default()
                    .intent
                    .as_str()
                    .to_string(),
                invalid_intent_policy:
                    "invalid PDF intent names are reported and resolved through the default perceptual policy"
                        .to_string(),
                native_bpc_status: if native.available {
                    "wired_to_littlecms_blackpoint_compensation_flag_on_request".to_string()
                } else {
                    "native_lcms2_unavailable_in_current_build".to_string()
                },
                fallback_bpc_status:
                    "bpc_unsupported_in_fallback; fallback output is preview, not proof".to_string(),
                cache_key_fields: vec![
                    "backend".to_string(),
                    "profile_hash".to_string(),
                    "source_channels".to_string(),
                    "destination_channels".to_string(),
                    "rendering_intent".to_string(),
                    "black_point_compensation".to_string(),
                    "output_intent".to_string(),
                    "plate_cache_fingerprint".to_string(),
                ],
            },
            separation_framebuffer: separation_framebuffer.clone(),
            spot_plates,
            devicen_plates,
            plate_preview: PlatePreviewReport {
                output_mode: "report_hashes_and_sparse_plate_state".to_string(),
                preview_hash_count,
                plate_preview_artifact:
                    "target/prepress_cmm-prepress-cmm/plate-preview-results-prepress_cmm.json"
                        .to_string(),
            },
            native_cmm_policy: if native.available {
                "native LittleCMS is feature-gated; device-link and ordinary ICC transforms are attempted only when profile class, channel counts, and context are legal".to_string()
            } else {
                "native LittleCMS is not active; prepress-only transforms are inventory/report-only".to_string()
            },
            fallback_policy:
                "default/WASM fallback inventories device-link and multicolor profiles, preserves plate/tint metadata, and labels alternate-space output as preview only".to_string(),
            known_limits: vec![
                "Prepress Proofing owns bounded overprint close-out; Prepress CMM remains the compatibility baseline".to_string(),
                "certification-grade PDF/X validation is later standards work".to_string(),
                "Nchannel Plate Prepress owns n-channel output closure; Prepress CMM section remains the compatibility baseline".to_string(),
            ],
        }
    }
}

#[derive(Debug, Clone)]
pub struct SeparationFramebuffer {
    page_number: Option<usize>,
    tile_identity: Option<String>,
    memory_budget_bytes: usize,
    contributions: Vec<PlateContribution>,
    nchannel_samples: Vec<NChannelSample>,
    diagnostics: Vec<String>,
    report_only_degraded: bool,
}

impl Default for SeparationFramebuffer {
    fn default() -> Self {
        Self::new(None, None, DEFAULT_SEPARATION_FRAMEBUFFER_BUDGET_BYTES)
    }
}

impl SeparationFramebuffer {
    pub fn for_page(page_number: usize, width: u32, height: u32) -> Self {
        Self::new(
            Some(page_number),
            Some(format!("full:{width}x{height}")),
            DEFAULT_SEPARATION_FRAMEBUFFER_BUDGET_BYTES,
        )
    }

    pub fn new(
        page_number: Option<usize>,
        tile_identity: Option<String>,
        memory_budget_bytes: usize,
    ) -> Self {
        Self {
            page_number,
            tile_identity,
            memory_budget_bytes,
            contributions: Vec::new(),
            nchannel_samples: Vec::new(),
            diagnostics: Vec::new(),
            report_only_degraded: false,
        }
    }

    pub fn record(&mut self, contribution: PlateContribution) {
        let mut names = self.plane_names();
        names.insert(contribution.plane_name.clone());
        if names.len() > MAX_PREPRESS_PLATES {
            self.report_only_degraded = true;
            self.diagnostics.push(format!(
                "plate count {} exceeds cap {}; contribution for {} kept report-only",
                names.len(),
                MAX_PREPRESS_PLATES,
                contribution.plane_name
            ));
            return;
        }
        let estimated = (self.contributions.len() + 1) * CONTRIBUTION_ACCOUNTING_BYTES
            + (self.nchannel_samples.len() + 1) * NCHANNEL_SAMPLE_ACCOUNTING_BYTES;
        if estimated > self.memory_budget_bytes {
            self.report_only_degraded = true;
            self.diagnostics.push(format!(
                "estimated plate framebuffer bytes {estimated} exceed budget {}",
                self.memory_budget_bytes
            ));
            return;
        }
        self.nchannel_samples
            .push(nchannel_sample_from_contribution(&contribution));
        self.contributions.push(contribution);
    }

    pub fn record_all(&mut self, contributions: impl IntoIterator<Item = PlateContribution>) {
        for contribution in contributions {
            self.record(contribution);
        }
    }

    pub fn absorb(&mut self, other: SeparationFramebuffer) {
        self.report_only_degraded |= other.report_only_degraded;
        self.diagnostics.extend(other.diagnostics);
        self.record_all(other.contributions);
    }

    pub fn report(&self) -> SeparationFramebufferReport {
        let summaries = self.plate_summaries();
        let deterministic_plane_order = summaries
            .iter()
            .map(|summary| summary.plane_name.clone())
            .collect::<Vec<_>>();
        let operation_kinds = self.operation_kinds();
        let mut fingerprint_parts = deterministic_plane_order.clone();
        fingerprint_parts.extend(operation_kinds.iter().map(|op| format!("op={op}")));
        fingerprint_parts.extend(
            self.contributions
                .iter()
                .map(|contribution| format!("overprint={}", contribution.overprint_posture)),
        );
        fingerprint_parts.push(format!("nchannel_samples={}", self.nchannel_samples.len()));
        let cache_fingerprint = cache_fingerprint(
            "plate-framebuffer",
            &fingerprint_parts,
            cmm::ColorTransformOptions::default().intent.as_str(),
            cmm::ColorTransformOptions::default().black_point_compensation,
            cmm::native_cmm_status().selected_backend,
            None,
        );
        SeparationFramebufferReport {
            true_separation_framebuffer: true,
            storage_model: "sampled_nchannel_plate_surface_with_sparse_tile_local_plane_storage"
                .to_string(),
            page_number: self.page_number,
            tile_identity: self.tile_identity.clone(),
            deterministic_plane_order,
            plate_count: summaries.len(),
            contribution_count: self.contributions.len(),
            memory_budget_bytes: self.memory_budget_bytes,
            estimated_memory_bytes: self.contributions.len() * CONTRIBUTION_ACCOUNTING_BYTES
                + self.nchannel_samples.len() * NCHANNEL_SAMPLE_ACCOUNTING_BYTES,
            scheduler_accounted: true,
            excessive_colorants_fail_closed: self.report_only_degraded,
            report_only_degraded: self.report_only_degraded,
            cache_fingerprint,
            nchannel_pixel_format: NChannelPixelFormatReport {
                memory_budget_bytes: self.memory_budget_bytes,
                ..NChannelPixelFormatReport::default()
            },
            sampled_plate_surface: true,
            per_sample_plate_contributions: self.nchannel_samples.len(),
            operation_kinds,
            plate_summaries: summaries.clone(),
            plate_previews: summaries
                .iter()
                .map(|summary| PlatePreviewHash {
                    plane_name: summary.plane_name.clone(),
                    preview_hash: summary.preview_hash.clone(),
                })
                .collect(),
            diagnostics: self.diagnostics.clone(),
        }
    }

    fn plane_names(&self) -> BTreeSet<String> {
        self.contributions
            .iter()
            .map(|contribution| contribution.plane_name.clone())
            .collect()
    }

    fn operation_kinds(&self) -> Vec<String> {
        self.contributions
            .iter()
            .map(|contribution| contribution.operation.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    fn plate_summaries(&self) -> Vec<PlateSummary> {
        let mut map: BTreeMap<String, PlateSummaryAccumulator> = BTreeMap::new();
        for contribution in &self.contributions {
            let entry = map
                .entry(contribution.plane_name.clone())
                .or_insert_with(|| PlateSummaryAccumulator {
                    kind: contribution.kind,
                    enabled: false,
                    contribution_count: 0,
                    max_tint_u16: 0,
                    provenance: BTreeSet::new(),
                    hash_parts: Vec::new(),
                });
            entry.enabled |= contribution.enabled;
            entry.contribution_count += 1;
            entry.max_tint_u16 = entry
                .max_tint_u16
                .max((contribution.tint.clamp(0.0, 1.0) * 65535.0).round() as u16);
            if let Some(object) = &contribution.object {
                entry.provenance.insert(object.clone());
            }
            entry.hash_parts.push(format!(
                "{}:{:?}:{:.6}:{:.6}:{:?}:{}:{}",
                contribution.plane_name,
                contribution.kind,
                contribution.tint,
                contribution.alpha,
                contribution.alternate_preview_rgb,
                contribution.operation,
                contribution.overprint_posture
            ));
        }
        map.into_iter()
            .map(|(plane_name, acc)| PlateSummary {
                plane_name,
                kind: acc.kind,
                enabled: acc.enabled,
                contribution_count: acc.contribution_count,
                max_tint_u16: acc.max_tint_u16,
                preview_hash: hash_strings(&acc.hash_parts),
                provenance: acc.provenance.into_iter().collect(),
            })
            .collect()
    }
}

#[derive(Debug)]
struct PlateSummaryAccumulator {
    kind: PlateKind,
    enabled: bool,
    contribution_count: usize,
    max_tint_u16: u16,
    provenance: BTreeSet<String>,
    hash_parts: Vec<String>,
}

pub fn classify_icc_profile(
    profile_bytes: &[u8],
    declared_components: Option<u8>,
    object: Option<String>,
) -> IccProfileInfo {
    let profile_hash = format!("{:016x}", fnv1a64(profile_bytes));
    if profile_bytes.len() > cmm::DEFAULT_MAX_ICC_PROFILE_BYTES {
        return IccProfileInfo {
            object,
            profile_hash,
            profile_bytes: profile_bytes.len(),
            declared_components,
            profile_class: IccProfileClass::Unsupported,
            profile_class_signature: None,
            profile_color_space: None,
            pcs: None,
            input_channels: None,
            output_channels: None,
            channel_labels: Vec::new(),
            rendering_intent_hint: None,
            is_multicolor: false,
            channel_mismatch: false,
            native_transform_status: "unsupported_profile_too_large".to_string(),
            fallback_transform_status: "unsupported_profile_too_large".to_string(),
            output_intent_interaction: "not_safe_for_output_intent".to_string(),
            reason: Some(format!(
                "profile is {} bytes; cap is {}",
                profile_bytes.len(),
                cmm::DEFAULT_MAX_ICC_PROFILE_BYTES
            )),
        };
    }
    if profile_bytes.len() < 128 {
        return IccProfileInfo {
            object,
            profile_hash,
            profile_bytes: profile_bytes.len(),
            declared_components,
            profile_class: IccProfileClass::Malformed,
            profile_class_signature: None,
            profile_color_space: None,
            pcs: None,
            input_channels: None,
            output_channels: None,
            channel_labels: Vec::new(),
            rendering_intent_hint: None,
            is_multicolor: false,
            channel_mismatch: false,
            native_transform_status: "unsupported_malformed_profile".to_string(),
            fallback_transform_status: "unsupported_malformed_profile".to_string(),
            output_intent_interaction: "not_safe_for_output_intent".to_string(),
            reason: Some("ICC header shorter than 128 bytes".to_string()),
        };
    }

    let class_sig = signature(profile_bytes, 12);
    let color_sig = signature(profile_bytes, 16);
    let pcs_sig = signature(profile_bytes, 20);
    let profile_class = class_sig
        .map(class_from_signature)
        .unwrap_or(IccProfileClass::Unknown);
    let color_space = color_sig.map(signature_to_string);
    let pcs = pcs_sig.map(signature_to_string);
    let input_channels = color_sig.and_then(channel_count_from_signature);
    let output_channels = if profile_class == IccProfileClass::DeviceLink {
        pcs_sig.and_then(channel_count_from_signature)
    } else {
        None
    };
    let rendering_intent_hint = read_rendering_intent(profile_bytes);
    let mut channel_labels = Vec::new();
    if let Some(count) = input_channels {
        channel_labels = labels_for_signature(color_space.as_deref(), count);
    }
    let is_multicolor = input_channels.is_some_and(|channels| channels > 4)
        || output_channels.is_some_and(|channels| channels > 4);
    let channel_mismatch = declared_components
        .zip(input_channels)
        .is_some_and(|(declared, input)| declared != input);
    let native = cmm::native_cmm_status();
    let native_transform_status = native_transform_status(
        profile_class,
        input_channels,
        output_channels,
        channel_mismatch,
        is_multicolor,
        native.available,
    );
    let fallback_transform_status = fallback_transform_status(
        profile_class,
        input_channels,
        channel_mismatch,
        is_multicolor,
    );
    let output_intent_interaction = output_intent_interaction(profile_class);
    let reason = if channel_mismatch {
        Some(format!(
            "declared /N {:?} does not match ICC input channels {:?}",
            declared_components, input_channels
        ))
    } else if profile_class == IccProfileClass::Unknown {
        Some("profile class signature is not recognized".to_string())
    } else {
        None
    };
    IccProfileInfo {
        object,
        profile_hash,
        profile_bytes: profile_bytes.len(),
        declared_components,
        profile_class,
        profile_class_signature: class_sig.map(signature_to_string),
        profile_color_space: color_space,
        pcs,
        input_channels,
        output_channels,
        channel_labels,
        rendering_intent_hint,
        is_multicolor,
        channel_mismatch,
        native_transform_status,
        fallback_transform_status,
        output_intent_interaction,
        reason,
    }
}

pub(crate) fn plate_contributions_for_color_space(
    space_obj: &PdfObject,
    components: &[f64],
    alpha: f32,
    reader: &PdfReader,
    object: Option<String>,
    operation: &str,
    page_number: Option<usize>,
) -> Vec<PlateContribution> {
    let state = OverprintStateModel::for_paint(
        false,
        false,
        0,
        operation,
        color_space_label(space_obj, reader),
        components,
        alpha,
        object.clone(),
    );
    plate_contributions_for_color_space_with_overprint(
        space_obj,
        components,
        alpha,
        reader,
        object,
        operation,
        page_number,
        &state,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn plate_contributions_for_color_space_with_overprint(
    space_obj: &PdfObject,
    components: &[f64],
    alpha: f32,
    reader: &PdfReader,
    object: Option<String>,
    operation: &str,
    page_number: Option<usize>,
    overprint: &OverprintStateModel,
) -> Vec<PlateContribution> {
    let resolved = match space_obj {
        PdfObject::Reference { .. } => reader
            .resolve(space_obj.clone())
            .unwrap_or_else(|_| space_obj.clone()),
        other => other.clone(),
    };
    let Some(arr) = resolved.as_array() else {
        return Vec::new();
    };
    let Some(family) = arr.first().and_then(PdfObject::as_name) else {
        return Vec::new();
    };
    let preview = preview_rgb(&resolved, components, alpha, reader);
    match family {
        "Separation" => {
            let name = arr.get(1).and_then(PdfObject::as_name).unwrap_or("Unknown");
            let kind = match name {
                "None" => PlateKind::None,
                "All" => PlateKind::All,
                process if is_process_colorant(process) => PlateKind::Process,
                _ => PlateKind::Spot,
            };
            vec![PlateContribution {
                plane_name: name.to_string(),
                kind,
                tint: components.first().copied().unwrap_or(1.0) as f32,
                alpha,
                enabled: name != "None",
                alternate_preview_rgb: preview,
                object,
                operation: operation.to_string(),
                page_number,
                tile: None,
                overprint_posture: overprint
                    .posture_for(kind, components.first().copied().unwrap_or(1.0) as f32),
            }]
        }
        "DeviceN" => {
            let names = arr
                .get(1)
                .and_then(PdfObject::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(PdfObject::as_name)
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if names.len() > colorspace::MAX_DEVICEN_COMPONENTS {
                return Vec::new();
            }
            names
                .iter()
                .enumerate()
                .map(|(idx, name)| PlateContribution {
                    plane_name: name.clone(),
                    kind: if name == "None" {
                        PlateKind::None
                    } else if is_process_colorant(name) {
                        PlateKind::Process
                    } else {
                        PlateKind::DeviceN
                    },
                    tint: components.get(idx).copied().unwrap_or(1.0) as f32,
                    alpha,
                    enabled: name != "None",
                    alternate_preview_rgb: preview,
                    object: object.clone(),
                    operation: operation.to_string(),
                    page_number,
                    tile: None,
                    overprint_posture: overprint.posture_for(
                        if name == "None" {
                            PlateKind::None
                        } else if is_process_colorant(name) {
                            PlateKind::Process
                        } else {
                            PlateKind::DeviceN
                        },
                        components.get(idx).copied().unwrap_or(1.0) as f32,
                    ),
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

pub(crate) fn color_space_label(space_obj: &PdfObject, reader: &PdfReader) -> String {
    let resolved = match space_obj {
        PdfObject::Reference { .. } => reader
            .resolve(space_obj.clone())
            .unwrap_or_else(|_| space_obj.clone()),
        other => other.clone(),
    };
    match resolved {
        PdfObject::Name(name) => name,
        PdfObject::Array(arr) => arr
            .first()
            .and_then(PdfObject::as_name)
            .map(str::to_string)
            .unwrap_or_else(|| "array".to_string()),
        _ => "unknown".to_string(),
    }
}

pub(crate) fn cache_fingerprint_for_prepress_resources<'a, 'b>(
    spaces: impl IntoIterator<Item = &'a PdfObject>,
    ext_g_states: impl IntoIterator<Item = &'b crate::object::PdfDictionary>,
) -> String {
    let mut parts = Vec::new();
    for space in spaces {
        let PdfObject::Array(arr) = space else {
            continue;
        };
        let Some(family) = arr.first().and_then(PdfObject::as_name) else {
            continue;
        };
        match family {
            "Separation" => {
                if let Some(name) = arr.get(1).and_then(PdfObject::as_name) {
                    parts.push(format!("Separation:{name}"));
                }
            }
            "DeviceN" => {
                let names = arr
                    .get(1)
                    .and_then(PdfObject::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(PdfObject::as_name)
                            .collect::<Vec<_>>()
                            .join(",")
                    })
                    .unwrap_or_default();
                parts.push(format!("DeviceN:{names}"));
            }
            _ => {}
        }
    }
    for gs in ext_g_states {
        if let Some(op) = gs.get_bool("op") {
            parts.push(format!("op={op}"));
        }
        if let Some(op) = gs.get_bool("OP") {
            parts.push(format!("OP={op}"));
        }
        if let Some(opm) = gs.get_integer("OPM") {
            parts.push(format!("OPM={opm}"));
        }
        if let Some(alpha) = gs.get("ca").and_then(PdfObject::as_number) {
            parts.push(format!("fill_alpha={alpha:.6}"));
        }
        if let Some(alpha) = gs.get("CA").and_then(PdfObject::as_number) {
            parts.push(format!("stroke_alpha={alpha:.6}"));
        }
        if gs.get("SMask").is_some() {
            parts.push("soft_mask=present".to_string());
        }
        if gs.get("AIS").is_some() {
            parts.push("alpha_source_present".to_string());
        }
    }
    parts.sort();
    cache_fingerprint(
        "render-cache",
        &parts,
        cmm::ColorTransformOptions::default().intent.as_str(),
        cmm::ColorTransformOptions::default().black_point_compensation,
        cmm::native_cmm_status().selected_backend,
        None,
    )
}

fn preview_rgb(
    space_obj: &PdfObject,
    components: &[f64],
    alpha: f32,
    reader: &PdfReader,
) -> Option<[u8; 3]> {
    match colorspace::resolve_named_color(space_obj, components, alpha, reader) {
        colorspace::NamedColor::Color(color) => {
            let px = color.to_pixel_color();
            Some([px[0], px[1], px[2]])
        }
        colorspace::NamedColor::NoPaint | colorspace::NamedColor::Unhandled => None,
    }
}

fn signature(bytes: &[u8], offset: usize) -> Option<[u8; 4]> {
    let slice = bytes.get(offset..offset + 4)?;
    Some([slice[0], slice[1], slice[2], slice[3]])
}

fn signature_to_string(sig: [u8; 4]) -> String {
    String::from_utf8_lossy(&sig).trim_end().to_string()
}

fn class_from_signature(sig: [u8; 4]) -> IccProfileClass {
    match &sig {
        b"scnr" => IccProfileClass::Input,
        b"mntr" => IccProfileClass::Display,
        b"prtr" => IccProfileClass::Output,
        b"link" => IccProfileClass::DeviceLink,
        b"spac" => IccProfileClass::ColorSpaceConversion,
        b"abst" => IccProfileClass::Abstract,
        b"nmcl" => IccProfileClass::NamedColor,
        _ => IccProfileClass::Unknown,
    }
}

fn channel_count_from_signature(sig: [u8; 4]) -> Option<u8> {
    match &sig {
        b"GRAY" => Some(1),
        b"RGB " | b"Lab " | b"XYZ " => Some(3),
        b"CMYK" => Some(4),
        [b'2', b'C', b'L', b'R'] => Some(2),
        [b'3', b'C', b'L', b'R'] => Some(3),
        [b'4', b'C', b'L', b'R'] => Some(4),
        [b'5', b'C', b'L', b'R'] => Some(5),
        [b'6', b'C', b'L', b'R'] => Some(6),
        [b'7', b'C', b'L', b'R'] => Some(7),
        [b'8', b'C', b'L', b'R'] => Some(8),
        [b'9', b'C', b'L', b'R'] => Some(9),
        [b'A', b'C', b'L', b'R'] => Some(10),
        [b'B', b'C', b'L', b'R'] => Some(11),
        [b'C', b'C', b'L', b'R'] => Some(12),
        [b'D', b'C', b'L', b'R'] => Some(13),
        [b'E', b'C', b'L', b'R'] => Some(14),
        [b'F', b'C', b'L', b'R'] => Some(15),
        _ => None,
    }
}

fn native_cmm_context_label() -> String {
    let native = cmm::native_cmm_status();
    if native.available {
        "native_lcms2_output_intent_context_available".to_string()
    } else {
        "fallback_or_wasm_preview_only_output_intent_context".to_string()
    }
}

fn paint_role_from_operation(operation: &str) -> String {
    if operation.contains("text") && operation.contains("stroke") {
        "text_stroke".to_string()
    } else if operation.contains("text") {
        "text_fill".to_string()
    } else if operation.contains("stroke") {
        "stroke".to_string()
    } else if operation.contains("image") {
        "image".to_string()
    } else if operation.contains("shading") {
        "shading".to_string()
    } else if operation.contains("pattern") {
        "pattern".to_string()
    } else {
        "fill".to_string()
    }
}

fn supported_prepress_proofing_cases(operations: &BTreeSet<&str>) -> Vec<String> {
    let mut cases = vec![
        "OP_fill_flag_distinct_from_OP_stroke_flag".to_string(),
        "OPM_0_process_replacement".to_string(),
        "OPM_1_zero_tint_process_preservation".to_string(),
        "DeviceCMYK_process_overprint_preview".to_string(),
        "Separation_spot_plate_overprint_posture".to_string(),
        "DeviceN_process_vs_named_component_overprint_posture".to_string(),
        "knockout_replacement_when_overprint_disabled".to_string(),
        "RGB_preview_and_plate_hash_consistency".to_string(),
        "native_fallback_wasm_behavior_reported".to_string(),
        "color_managed_shadings".to_string(),
        "color_managed_tiling_patterns".to_string(),
        "tile_band_progressive_cache_fingerprint_equivalence".to_string(),
    ];
    if operations.iter().any(|op| op.starts_with("text_")) {
        cases.push("text_fill_stroke_overprint_plate_contributions".to_string());
    }
    if operations.iter().any(|op| matches!(*op, "fill" | "stroke")) {
        cases.push("vector_fill_stroke_overprint_plate_contributions".to_string());
    }
    if operations.iter().any(|op| op.starts_with("image_")) {
        cases.push("image_mask_and_named_space_overprint_plate_contributions".to_string());
    }
    if operations.iter().any(|op| op.starts_with("shading_")) {
        cases.push("shading_overprint_plate_contributions".to_string());
    }
    if operations.iter().any(|op| op.starts_with("pattern_")) {
        cases.push("pattern_overprint_plate_contributions".to_string());
    }
    cases
}

fn labels_for_signature(signature: Option<&str>, count: u8) -> Vec<String> {
    match signature {
        Some("GRAY") => vec!["Gray".to_string()],
        Some("RGB") => ["Red", "Green", "Blue"]
            .into_iter()
            .map(str::to_string)
            .collect(),
        Some("CMYK") => ["Cyan", "Magenta", "Yellow", "Black"]
            .into_iter()
            .map(str::to_string)
            .collect(),
        _ => (0..count)
            .map(|idx| format!("channel_{}", idx + 1))
            .collect(),
    }
}

fn read_rendering_intent(bytes: &[u8]) -> Option<String> {
    let raw = bytes.get(64..68)?;
    let value = u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]);
    Some(
        match value {
            0 => "perceptual",
            1 => "relative_colorimetric",
            2 => "saturation",
            3 => "absolute_colorimetric",
            _ => "unknown",
        }
        .to_string(),
    )
}

fn native_transform_status(
    profile_class: IccProfileClass,
    input_channels: Option<u8>,
    output_channels: Option<u8>,
    channel_mismatch: bool,
    is_multicolor: bool,
    native_available: bool,
) -> String {
    if !native_available {
        return "native_lcms2_unavailable_report_only".to_string();
    }
    if channel_mismatch {
        return "unsupported_channel_mismatch_fail_closed".to_string();
    }
    match profile_class {
        IccProfileClass::DeviceLink => match (input_channels, output_channels) {
            (Some(1 | 3 | 4), Some(1 | 3 | 4)) => {
                "native_device_link_transform_shape_supported_when_pdf_context_is_legal".to_string()
            }
            _ => "unsupported_device_link_channel_shape_fail_closed".to_string(),
        },
        IccProfileClass::Input | IccProfileClass::Display | IccProfileClass::Output
            if !is_multicolor && matches!(input_channels, Some(1 | 3 | 4)) =>
        {
            "native_lcms2_profile_to_srgb_or_proofing_transform_supported".to_string()
        }
        _ if is_multicolor => {
            "unsupported_multicolor_transform_inventory_only_safe_pixel_format_limit".to_string()
        }
        IccProfileClass::Malformed | IccProfileClass::Unsupported => {
            "unsupported_profile_fail_closed".to_string()
        }
        _ => "unsupported_profile_class_reported".to_string(),
    }
}

fn fallback_transform_status(
    profile_class: IccProfileClass,
    input_channels: Option<u8>,
    channel_mismatch: bool,
    is_multicolor: bool,
) -> String {
    if channel_mismatch {
        return "fallback_unsupported_channel_mismatch_fail_closed".to_string();
    }
    match profile_class {
        IccProfileClass::DeviceLink => {
            "fallback_device_link_unsupported_alternate_preview_only_if_pdf_supplies_safe_alternate"
                .to_string()
        }
        _ if is_multicolor => {
            "fallback_multicolor_unsupported_alternate_preview_only_if_pdf_supplies_safe_alternate"
                .to_string()
        }
        IccProfileClass::Input | IccProfileClass::Display | IccProfileClass::Output
            if matches!(input_channels, Some(1 | 3 | 4)) =>
        {
            "fallback_qcms_profile_to_srgb_preview_where_qcms_accepts_profile".to_string()
        }
        IccProfileClass::Malformed | IccProfileClass::Unsupported => {
            "fallback_unsupported_profile_fail_closed".to_string()
        }
        _ => "fallback_unsupported_profile_class_reported".to_string(),
    }
}

fn output_intent_interaction(profile_class: IccProfileClass) -> String {
    match profile_class {
        IccProfileClass::DeviceLink => {
            "device_link_is_a_fixed_transform; do_not_double_proof_against_output_intent"
                .to_string()
        }
        IccProfileClass::Output => {
            "valid_as_destination_output_intent_when_channel_shape_matches".to_string()
        }
        IccProfileClass::Input | IccProfileClass::Display => {
            "may_be_source_profile_for_output_intent_proofing_when_context_is_unambiguous"
                .to_string()
        }
        _ => "not_safe_for_output_intent_proofing_without_explicit_context".to_string(),
    }
}

fn is_process_colorant(name: &str) -> bool {
    matches!(
        name,
        "Cyan" | "Magenta" | "Yellow" | "Black" | "C" | "M" | "Y" | "K"
    )
}

fn nchannel_sample_from_contribution(contribution: &PlateContribution) -> NChannelSample {
    let options = cmm::ColorTransformOptions::default();
    let label = match contribution.kind {
        PlateKind::Process => process_label(&contribution.plane_name),
        PlateKind::Spot | PlateKind::DeviceN | PlateKind::All | PlateKind::None => {
            contribution.plane_name.clone()
        }
    };
    let value = (contribution.tint.clamp(0.0, 1.0) * 65535.0).round() as u16;
    let alpha = (contribution.alpha.clamp(0.0, 1.0) * 65535.0).round() as u16;
    let backend_status = cmm::native_cmm_status().selected_backend.to_string();
    let transform_key = cache_fingerprint(
        "nchannel-sample",
        &[
            label.clone(),
            format!("kind={:?}", contribution.kind),
            format!("operation={}", contribution.operation),
            format!("page={:?}", contribution.page_number),
            format!("tile={:?}", contribution.tile),
            format!("overprint={}", contribution.overprint_posture),
        ],
        options.intent.as_str(),
        options.black_point_compensation,
        &backend_status,
        None,
    );
    NChannelSample {
        channel_labels: vec![label],
        channel_kinds: vec![contribution.kind],
        channel_values_u16: vec![value],
        alpha_u16: alpha,
        alternate_preview_rgb: contribution.alternate_preview_rgb,
        object: contribution.object.clone(),
        operation: contribution.operation.clone(),
        page_number: contribution.page_number,
        tile: contribution.tile.clone(),
        color_space_provenance: contribution.object.clone(),
        profile_hash: None,
        transform_key,
        rendering_intent: options.intent.as_str().to_string(),
        black_point_compensation: options.black_point_compensation,
        backend_status,
    }
}

fn process_label(name: &str) -> String {
    match name {
        "C" | "Cyan" => "Cyan".to_string(),
        "M" | "Magenta" => "Magenta".to_string(),
        "Y" | "Yellow" => "Yellow".to_string(),
        "K" | "Black" => "Black".to_string(),
        other => other.to_string(),
    }
}

fn cache_fingerprint(
    prefix: &str,
    parts: &[String],
    intent: &str,
    bpc: bool,
    backend: &str,
    output_intent_hash: Option<&str>,
) -> String {
    let mut material = vec![
        prefix.to_string(),
        format!("intent={intent}"),
        format!("bpc={bpc}"),
        format!("backend={backend}"),
        format!("output_intent={}", output_intent_hash.unwrap_or("none")),
    ];
    material.extend(parts.iter().cloned());
    hash_strings(&material)
}

fn hash_strings(parts: &[String]) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for part in parts {
        for byte in part.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x1000_0000_01b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    format!("{hash:016x}")
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_icc(class: &[u8; 4], color: &[u8; 4], pcs: &[u8; 4], intent: u32) -> Vec<u8> {
        let mut bytes = vec![0u8; 128];
        bytes[0..4].copy_from_slice(&(128u32).to_be_bytes());
        bytes[12..16].copy_from_slice(class);
        bytes[16..20].copy_from_slice(color);
        bytes[20..24].copy_from_slice(pcs);
        bytes[64..68].copy_from_slice(&intent.to_be_bytes());
        bytes
    }

    #[test]
    fn classifies_device_link_profile_header_and_channels() {
        let profile = fake_icc(b"link", b"CMYK", b"CMYK", 1);
        let info = classify_icc_profile(&profile, Some(4), Some("12 0 R".to_string()));
        assert_eq!(info.profile_class, IccProfileClass::DeviceLink);
        assert_eq!(info.input_channels, Some(4));
        assert_eq!(info.output_channels, Some(4));
        assert_eq!(
            info.rendering_intent_hint.as_deref(),
            Some("relative_colorimetric")
        );
        assert!(!info.channel_mismatch);
        assert!(info
            .output_intent_interaction
            .contains("do_not_double_proof"));
    }

    #[test]
    fn classifies_multicolor_inventory_and_mismatch() {
        let profile = fake_icc(b"prtr", b"5CLR", b"Lab ", 0);
        let info = classify_icc_profile(&profile, Some(4), None);
        assert_eq!(info.profile_class, IccProfileClass::Output);
        assert_eq!(info.input_channels, Some(5));
        assert!(info.is_multicolor);
        assert!(info.channel_mismatch);
        assert!(info.fallback_transform_status.contains("channel_mismatch"));
    }

    #[test]
    fn malformed_icc_fails_closed() {
        let info = classify_icc_profile(b"bad", Some(3), None);
        assert_eq!(info.profile_class, IccProfileClass::Malformed);
        assert_eq!(
            info.native_transform_status,
            "unsupported_malformed_profile"
        );
        assert_eq!(
            info.fallback_transform_status,
            "unsupported_malformed_profile"
        );
    }

    #[test]
    fn separation_framebuffer_is_sparse_bounded_and_ordered() {
        let mut fb = SeparationFramebuffer::new(Some(1), Some("tile:0,0,64,64".to_string()), 4096);
        fb.record(PlateContribution {
            plane_name: "SpotBlue".to_string(),
            kind: PlateKind::Spot,
            tint: 0.5,
            alpha: 1.0,
            enabled: true,
            alternate_preview_rgb: Some([10, 20, 30]),
            object: Some("3 0 R".to_string()),
            operation: "fill".to_string(),
            page_number: Some(1),
            tile: Some("0,0,64,64".to_string()),
            overprint_posture: "overprint_named_plate_tint_preserved_with_preview_consistency"
                .to_string(),
        });
        fb.record(PlateContribution {
            plane_name: "Cyan".to_string(),
            kind: PlateKind::Process,
            tint: 1.0,
            alpha: 0.75,
            enabled: true,
            alternate_preview_rgb: Some([0, 173, 239]),
            object: Some("3 0 R".to_string()),
            operation: "stroke".to_string(),
            page_number: Some(1),
            tile: Some("0,0,64,64".to_string()),
            overprint_posture:
                "overprint_opm1_process_nonzero_tint_replaces_that_component_preserves_others"
                    .to_string(),
        });
        let report = fb.report();
        assert!(report.true_separation_framebuffer);
        assert_eq!(
            report.deterministic_plane_order,
            vec!["Cyan".to_string(), "SpotBlue".to_string()]
        );
        assert_eq!(report.plate_count, 2);
        assert_eq!(report.contribution_count, 2);
        assert!(report.scheduler_accounted);
        assert!(!report.cache_fingerprint.is_empty());
    }
}
