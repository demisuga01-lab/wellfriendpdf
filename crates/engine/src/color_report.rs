//! Structured color/prepress inventory and validation reporting.
//!
//! The renderer's conversion code lives under `render::cmm`,
//! `render::colorspace`, and `render::function`. This module is the inspectable
//! report surface for those decisions: which PDF color spaces are present, which
//! ICC/output-intent resources exist, whether spot/DeviceN/overprint features
//! are being approximated for screen preview, and which caps protect color
//! transforms.

use crate::content::ContentParser;
use crate::filters::{decode_stream_lossless, StreamDecodeStatus};
use crate::object::{PdfDictionary, PdfObject};
use crate::prepress::{self, IccProfileClass, Prompt12PrepressReport, SeparationFramebuffer};
use crate::reader::PdfReader;
use crate::render::{cmm, colorspace, function};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorValidationProfile {
    Generic,
    PdfA,
    PdfX,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorSeverity {
    Info,
    Warning,
    Error,
    SecurityLimit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColorDiagnostic {
    pub code: String,
    pub severity: ColorSeverity,
    pub message: String,
    pub object: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColorLimits {
    pub max_icc_profile_bytes: usize,
    pub max_type0_sample_values: usize,
    pub max_type4_tokens: usize,
    pub max_type4_stack: usize,
    pub max_devicen_components: usize,
    pub max_prepress_plates: usize,
    pub separation_framebuffer_memory_budget_bytes: usize,
    pub max_content_stream_color_scan_bytes: usize,
}

impl Default for ColorLimits {
    fn default() -> Self {
        Self {
            max_icc_profile_bytes: cmm::DEFAULT_MAX_ICC_PROFILE_BYTES,
            max_type0_sample_values: function::MAX_TYPE0_SAMPLE_VALUES,
            max_type4_tokens: function::MAX_TYPE4_TOKENS,
            max_type4_stack: function::MAX_TYPE4_STACK,
            max_devicen_components: colorspace::MAX_DEVICEN_COMPONENTS,
            max_prepress_plates: prepress::MAX_PREPRESS_PLATES,
            separation_framebuffer_memory_budget_bytes:
                prepress::DEFAULT_SEPARATION_FRAMEBUFFER_BUDGET_BYTES,
            max_content_stream_color_scan_bytes: 4 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColorBackendDecision {
    pub outcome: String,
    pub default_backend: String,
    pub native_littlecms_integrated: bool,
    pub native_littlecms_compiled: bool,
    pub native_littlecms_available: bool,
    pub native_littlecms_version: Option<String>,
    pub feature_flag: String,
    pub backend_selected: String,
    pub default_build_unsafe_ffi: bool,
    pub icc_backend: String,
    pub supported_rendering_intents: Vec<String>,
    pub device_cmyk_preview: String,
    pub bpc_support: String,
    pub native_littlecms_decision: String,
    pub native_boundary: String,
    pub linking_posture: String,
}

impl Default for ColorBackendDecision {
    fn default() -> Self {
        let native = cmm::native_cmm_status();
        Self {
            outcome: if native.available {
                "Prompt 11B: feature-gated LittleCMS/lcms2 native CMM backend active"
                    .to_string()
            } else {
                "B: safe Rust/qcms accurate-enough preview backend".to_string()
            },
            default_backend: "portable-qcms-fallback".to_string(),
            native_littlecms_integrated: native.compiled,
            native_littlecms_compiled: native.compiled,
            native_littlecms_available: native.available,
            native_littlecms_version: native.native_version,
            feature_flag: native.feature_flag.to_string(),
            backend_selected: native.selected_backend.to_string(),
            default_build_unsafe_ffi: false,
            icc_backend: if native.available {
                "lcms2 for ICCBased Gray/RGB/CMYK profile-to-sRGB preview transforms; qcms remains the default portable fallback".to_string()
            } else {
                "qcms for ICCBased profile-to-sRGB preview transforms".to_string()
            },
            supported_rendering_intents: cmm::SUPPORTED_NATIVE_LCMS2_INTENTS
                .iter()
                .map(|intent| intent.as_str().to_string())
                .collect(),
            device_cmyk_preview:
                "deterministic Poppler/Splash-like process-ink interpolation; CMYK fill overprint preview preserves zero-ink channels in the RGB framebuffer approximation"
                    .to_string(),
            bpc_support: if native.available {
                "implemented for LittleCMS transforms through cmsFLAGS_BLACKPOINTCOMPENSATION when requested; default qcms fallback reports BPC unsupported".to_string()
            } else {
                "reported and carried in options; qcms/default fallback does not implement black-point compensation".to_string()
            },
            native_littlecms_decision: if native.available {
                "implemented behind native-cmm-lcms2; default/WASM builds remain portable and do not link lcms2".to_string()
            } else {
                "available only when the explicit native-cmm-lcms2 feature is compiled on a native target; default/WASM builds use qcms fallback".to_string()
            },
            native_boundary: native.unsafe_boundary.to_string(),
            linking_posture: native.linking_posture.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColorSpaceUsage {
    pub family: String,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputIntentInfo {
    pub s: Option<String>,
    pub output_condition_identifier: Option<String>,
    pub output_condition: Option<String>,
    pub registry_name: Option<String>,
    pub dest_output_profile_present: bool,
    pub dest_output_profile_n: Option<i64>,
    pub dest_output_profile_bytes: Option<usize>,
    pub dest_output_profile_hash: Option<String>,
    pub dest_output_profile_class: Option<IccProfileClass>,
    pub dest_output_profile_color_space: Option<String>,
    pub dest_output_profile_pcs: Option<String>,
    pub dest_output_profile_input_channels: Option<u8>,
    pub dest_output_profile_output_channels: Option<u8>,
    pub dest_output_profile_valid_icc: Option<bool>,
    pub dest_output_profile_valid_native_lcms2: Option<bool>,
    pub proofing_backend: String,
    pub proofing_status: String,
    pub proofing_rendering_intent: String,
    pub proofing_black_point_compensation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct OverprintReport {
    pub stroke_overprint_used: bool,
    pub fill_overprint_used: bool,
    pub overprint_mode_one_used: bool,
    pub ext_gstate_count: usize,
    pub cmyk_fill_preview_supported: bool,
    pub true_separation_framebuffer: bool,
    pub approximation_diagnostics: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SpotPreviewReport {
    pub separation_spaces: usize,
    pub devicen_spaces: usize,
    pub separation_tint_transforms: usize,
    pub devicen_tint_transforms: usize,
    pub missing_tint_transforms: usize,
    pub preview_model: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IccTransformCacheReport {
    pub hits: usize,
    pub misses: usize,
    pub evictions: usize,
    pub entries: usize,
    pub max_entries: usize,
    pub invalid_profiles: usize,
    pub unsupported_profiles: usize,
    pub native_lcms2_transforms: usize,
    pub native_lcms2_failures: usize,
    pub fallback_qcms_transforms: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IccFidelityVector {
    pub name: String,
    pub backend: String,
    pub input_rgb: Vec<u8>,
    pub output_rgb: Vec<u8>,
    pub max_abs_error: u8,
    pub tolerance: u8,
    pub passed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StandardsColorReport {
    pub scope: String,
    pub output_intent_checked: bool,
    pub icc_profile_checked: bool,
    pub device_color_policy_checked: bool,
    pub external_validator: Option<String>,
}

fn cache_report(metrics: cmm::IccTransformCacheMetrics) -> IccTransformCacheReport {
    IccTransformCacheReport {
        hits: metrics.hits,
        misses: metrics.misses,
        evictions: metrics.evictions,
        entries: metrics.entries,
        max_entries: metrics.max_entries,
        invalid_profiles: metrics.invalid_profiles,
        unsupported_profiles: metrics.unsupported_profiles,
        native_lcms2_transforms: metrics.native_lcms2_transforms,
        native_lcms2_failures: metrics.native_lcms2_failures,
        fallback_qcms_transforms: metrics.fallback_qcms_transforms,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColorReport {
    pub validation_profile: ColorValidationProfile,
    pub backend: ColorBackendDecision,
    pub limits: ColorLimits,
    pub icc_transform_cache: IccTransformCacheReport,
    pub icc_fidelity_vectors: Vec<IccFidelityVector>,
    pub color_spaces: Vec<ColorSpaceUsage>,
    pub spot_colorants: Vec<String>,
    pub devicen_components: Vec<Vec<String>>,
    pub spot_preview: SpotPreviewReport,
    pub output_intents: Vec<OutputIntentInfo>,
    pub rendering_intents: Vec<ColorSpaceUsage>,
    pub overprint: OverprintReport,
    pub prompt12_prepress_cmm_device_link_separation_plates: Prompt12PrepressReport,
    pub standards: StandardsColorReport,
    pub diagnostics: Vec<ColorDiagnostic>,
}

impl ColorReport {
    fn new(validation_profile: ColorValidationProfile) -> Self {
        Self {
            validation_profile,
            backend: ColorBackendDecision::default(),
            limits: ColorLimits::default(),
            icc_transform_cache: cache_report(cmm::icc_transform_cache_metrics()),
            icc_fidelity_vectors: cmm::srgb_identity_fidelity_probes()
                .into_iter()
                .map(|probe| IccFidelityVector {
                    name: probe.name.to_string(),
                    backend: probe.backend.to_string(),
                    input_rgb: probe.input,
                    output_rgb: probe.output,
                    max_abs_error: probe.max_abs_error,
                    tolerance: probe.tolerance,
                    passed: probe.passed,
                })
                .collect(),
            color_spaces: Vec::new(),
            spot_colorants: Vec::new(),
            devicen_components: Vec::new(),
            spot_preview: SpotPreviewReport {
                preview_model: "PDF tint transform to alternate color space plus Prompt 12 sparse plate framebuffer; RGB output remains preview, not press proof".to_string(),
                ..SpotPreviewReport::default()
            },
            output_intents: Vec::new(),
            rendering_intents: Vec::new(),
            overprint: OverprintReport::default(),
            prompt12_prepress_cmm_device_link_separation_plates: Prompt12PrepressReport::default(),
            standards: StandardsColorReport {
                scope: "color-only OutputIntent/ICC/device-color/prepress checks; full PDF/A/PDF/X validation remains the compliance module/Prompt 09 scope".to_string(),
                output_intent_checked: false,
                icc_profile_checked: false,
                device_color_policy_checked: false,
                external_validator: None,
            },
            diagnostics: Vec::new(),
        }
    }
}

#[derive(Default)]
struct ColorReportBuilder {
    color_spaces: BTreeMap<String, usize>,
    spot_colorants: BTreeSet<String>,
    devicen_components: BTreeSet<Vec<String>>,
    rendering_intents: BTreeMap<String, usize>,
    diagnostics: Vec<ColorDiagnostic>,
    overprint: OverprintReport,
    spot_preview: SpotPreviewReport,
    icc_profiles: Vec<prepress::IccProfileInfo>,
    separation_framebuffer: SeparationFramebuffer,
}

impl ColorReportBuilder {
    fn finish(self, mut report: ColorReport) -> ColorReport {
        let prompt12_profiles = self.icc_profiles.clone();
        let prompt12_framebuffer = self.separation_framebuffer.report();
        let device_rgb_used = self.color_spaces.contains_key("DeviceRGB");
        report.color_spaces = self
            .color_spaces
            .into_iter()
            .map(|(family, count)| ColorSpaceUsage { family, count })
            .collect();
        report.spot_colorants = self.spot_colorants.into_iter().collect();
        report.devicen_components = self.devicen_components.into_iter().collect();
        report.rendering_intents = self
            .rendering_intents
            .into_iter()
            .map(|(family, count)| ColorSpaceUsage { family, count })
            .collect();
        report.spot_preview = SpotPreviewReport {
            preview_model: report.spot_preview.preview_model,
            ..self.spot_preview
        };
        report.overprint = self.overprint;
        report.overprint.true_separation_framebuffer =
            prompt12_framebuffer.true_separation_framebuffer;
        report.prompt12_prepress_cmm_device_link_separation_plates =
            Prompt12PrepressReport::from_parts(prompt12_profiles, prompt12_framebuffer);
        report.diagnostics.extend(self.diagnostics);
        if matches!(report.validation_profile, ColorValidationProfile::PdfX) && device_rgb_used {
            report.diagnostics.push(ColorDiagnostic {
                code: "color.pdfx.device_rgb.warning".to_string(),
                severity: ColorSeverity::Warning,
                message: "PDF/X color validation found DeviceRGB usage; prepress output should be CMYK/spot or explicitly color managed".to_string(),
                object: None,
            });
        }
        report.icc_transform_cache = cache_report(cmm::icc_transform_cache_metrics());
        report.standards.device_color_policy_checked = matches!(
            report.validation_profile,
            ColorValidationProfile::PdfA | ColorValidationProfile::PdfX
        );
        report
    }

    fn count_space(&mut self, family: impl Into<String>) {
        *self.color_spaces.entry(family.into()).or_insert(0) += 1;
    }

    fn count_intent(&mut self, intent: impl Into<String>) {
        *self.rendering_intents.entry(intent.into()).or_insert(0) += 1;
    }

    fn diagnostic(
        &mut self,
        code: &str,
        severity: ColorSeverity,
        message: impl Into<String>,
        object: Option<String>,
    ) {
        self.diagnostics.push(ColorDiagnostic {
            code: code.to_string(),
            severity,
            message: message.into(),
            object,
        });
    }
}

pub fn color_report_bytes(
    bytes: &[u8],
    validation_profile: ColorValidationProfile,
) -> crate::Result<ColorReport> {
    let reader = PdfReader::from_bytes(bytes.to_vec())?;
    Ok(color_report(&reader, validation_profile))
}

pub fn color_report(reader: &PdfReader, validation_profile: ColorValidationProfile) -> ColorReport {
    let mut report = ColorReport::new(validation_profile);
    let mut builder = ColorReportBuilder::default();

    report.output_intents = parse_output_intents(reader, validation_profile, &mut builder);
    report.standards.output_intent_checked = matches!(
        validation_profile,
        ColorValidationProfile::PdfA | ColorValidationProfile::PdfX
    );
    report.standards.icc_profile_checked = !report.output_intents.is_empty();
    if report.output_intents.is_empty()
        && matches!(
            validation_profile,
            ColorValidationProfile::PdfA | ColorValidationProfile::PdfX
        )
    {
        builder.diagnostic(
            "color.output_intent.missing",
            ColorSeverity::Error,
            format!("{validation_profile:?} color validation requires an OutputIntent"),
            None,
        );
    }

    for (number, generation) in reader.object_ids() {
        let object_label = format!("{number} {generation} R");
        if let Ok(object) = reader.get_object(number, generation) {
            scan_object(&object, reader, &mut builder, Some(object_label), 0);
        }
    }

    builder.finish(report)
}

fn parse_output_intents(
    reader: &PdfReader,
    validation_profile: ColorValidationProfile,
    builder: &mut ColorReportBuilder,
) -> Vec<OutputIntentInfo> {
    let Some(root) = catalog_dict(reader) else {
        return Vec::new();
    };
    let Some(raw) = root.get("OutputIntents") else {
        return Vec::new();
    };
    let intents: Vec<PdfObject> = match resolve_object(raw, reader) {
        Some(PdfObject::Array(items)) => items,
        Some(other) => vec![other],
        None => return Vec::new(),
    };
    intents
        .iter()
        .filter_map(|intent| {
            let native = cmm::native_cmm_status();
            let dict = resolve_to_dict(intent, reader)?;
            let mut info = OutputIntentInfo {
                s: dict.get_name("S").map(str::to_string),
                output_condition_identifier: dict
                    .get("OutputConditionIdentifier")
                    .and_then(as_pdf_string),
                output_condition: dict.get("OutputCondition").and_then(as_pdf_string),
                registry_name: dict.get("RegistryName").and_then(as_pdf_string),
                dest_output_profile_present: false,
                dest_output_profile_n: None,
                dest_output_profile_bytes: None,
                dest_output_profile_hash: None,
                dest_output_profile_class: None,
                dest_output_profile_color_space: None,
                dest_output_profile_pcs: None,
                dest_output_profile_input_channels: None,
                dest_output_profile_output_channels: None,
                dest_output_profile_valid_icc: None,
                dest_output_profile_valid_native_lcms2: None,
                proofing_backend: native.selected_backend.to_string(),
                proofing_status: if native.available {
                    "pending_profile_validation".to_string()
                } else {
                    "native_lcms2_unavailable_fallback_report_only".to_string()
                },
                proofing_rendering_intent: cmm::ColorTransformOptions::default()
                    .intent
                    .as_str()
                    .to_string(),
                proofing_black_point_compensation: false,
            };
            if let Some(profile_obj) = dict.get("DestOutputProfile") {
                if let Some((profile_dict, stream)) = resolve_stream(profile_obj, reader) {
                    info.dest_output_profile_present = true;
                    info.dest_output_profile_n = profile_dict.get_integer("N");
                    match decode_stream_lossless(&stream, reader) {
                        Ok(decoded) if decoded.status == StreamDecodeStatus::Complete => {
                            info.dest_output_profile_bytes = Some(decoded.data.len());
                            if decoded.data.len() > cmm::DEFAULT_MAX_ICC_PROFILE_BYTES {
                                builder.diagnostic(
                                    "color.icc.profile_too_large",
                                    ColorSeverity::SecurityLimit,
                                    format!(
                                        "ICC profile is {} bytes; cap is {}",
                                        decoded.data.len(),
                                        cmm::DEFAULT_MAX_ICC_PROFILE_BYTES
                                    ),
                                    None,
                                );
                            } else {
                                let profile_info = prepress::classify_icc_profile(
                                    &decoded.data,
                                    info.dest_output_profile_n
                                        .and_then(|n| u8::try_from(n).ok()),
                                    Some("Catalog/OutputIntents/DestOutputProfile".to_string()),
                                );
                                info.dest_output_profile_hash =
                                    Some(profile_info.profile_hash.clone());
                                info.dest_output_profile_class =
                                    Some(profile_info.profile_class);
                                info.dest_output_profile_color_space =
                                    profile_info.profile_color_space.clone();
                                info.dest_output_profile_pcs = profile_info.pcs.clone();
                                info.dest_output_profile_input_channels =
                                    profile_info.input_channels;
                                info.dest_output_profile_output_channels =
                                    profile_info.output_channels;
                                push_profile_diagnostics(builder, &profile_info);
                                builder.icc_profiles.push(profile_info);
                                info.dest_output_profile_valid_icc = Some(
                                    qcms::Profile::new_from_slice(&decoded.data, false).is_some(),
                                );
                                info.dest_output_profile_valid_native_lcms2 =
                                    cmm::native_lcms2_profile_valid_for_components(
                                        &decoded.data,
                                        info.dest_output_profile_n
                                            .and_then(|n| u8::try_from(n).ok()),
                                    );
                                info.proofing_status =
                                    match info.dest_output_profile_valid_native_lcms2 {
                                        Some(true) => {
                                            let probe = [16, 32, 64, 96, 128, 160, 224, 240, 250];
                                            if cmm::proof_srgb_via_output_intent(
                                                &decoded.data,
                                                &probe,
                                                cmm::ColorTransformOptions::default(),
                                            )
                                            .is_some()
                                            {
                                                "native_lcms2_soft_proofing_available".to_string()
                                            } else {
                                                "native_lcms2_profile_valid_transform_unavailable"
                                                    .to_string()
                                            }
                                        }
                                        Some(false) => {
                                            "native_lcms2_invalid_or_channel_mismatch".to_string()
                                        }
                                        None => {
                                            "native_lcms2_unavailable_fallback_report_only"
                                                .to_string()
                                        }
                                    };
                                if info.dest_output_profile_valid_icc == Some(false) {
                                    builder.diagnostic(
                                        "color.icc.invalid_profile",
                                        ColorSeverity::Warning,
                                        "DestOutputProfile could not be parsed as ICC",
                                        None,
                                    );
                                }
                            }
                        }
                        _ => builder.diagnostic(
                            "color.icc.decode_failed",
                            ColorSeverity::Warning,
                            "DestOutputProfile stream could not be losslessly decoded",
                            None,
                        ),
                    }
                } else {
                    builder.diagnostic(
                        "color.output_intent.profile_missing",
                        ColorSeverity::Error,
                        "OutputIntent references a non-stream DestOutputProfile",
                        None,
                    );
                }
            } else if matches!(
                validation_profile,
                ColorValidationProfile::PdfA | ColorValidationProfile::PdfX
            ) {
                builder.diagnostic(
                    "color.output_intent.profile_missing",
                    ColorSeverity::Error,
                    "OutputIntent lacks DestOutputProfile",
                    None,
                );
            }
            match validation_profile {
                ColorValidationProfile::PdfA if info.s.as_deref() != Some("GTS_PDFA1") => {
                    builder.diagnostic(
                        "color.pdfa.output_intent.s",
                        ColorSeverity::Error,
                        format!(
                            "PDF/A color validation expects OutputIntent /S /GTS_PDFA1, got {:?}",
                            info.s
                        ),
                        None,
                    );
                }
                ColorValidationProfile::PdfX if info.s.as_deref() != Some("GTS_PDFX") => {
                    builder.diagnostic(
                        "color.pdfx.output_intent.s",
                        ColorSeverity::Error,
                        format!(
                            "PDF/X color validation expects OutputIntent /S /GTS_PDFX, got {:?}",
                            info.s
                        ),
                        None,
                    );
                }
                _ => {}
            }
            if matches!(validation_profile, ColorValidationProfile::PdfX)
                && info.dest_output_profile_n.is_some()
                && info.dest_output_profile_n != Some(4)
            {
                builder.diagnostic(
                    "color.pdfx.output_profile.not_cmyk",
                    ColorSeverity::Warning,
                    format!(
                        "PDF/X prepress color check expected a CMYK output profile (/N 4), got {:?}",
                        info.dest_output_profile_n
                    ),
                    None,
                );
            }
            Some(info)
        })
        .collect()
}

fn catalog_dict(reader: &PdfReader) -> Option<PdfDictionary> {
    let (number, generation) = reader.root_reference()?;
    match reader.get_object(number, generation).ok()? {
        PdfObject::Dictionary(dict) => Some(dict),
        _ => None,
    }
}

fn scan_object(
    object: &PdfObject,
    reader: &PdfReader,
    builder: &mut ColorReportBuilder,
    object_label: Option<String>,
    depth: usize,
) {
    if depth > 4 {
        return;
    }
    match object {
        PdfObject::Dictionary(dict) => scan_dict(dict, reader, builder, object_label, depth),
        PdfObject::Stream { dict, .. } => {
            scan_dict(dict, reader, builder, object_label.clone(), depth);
            scan_content_stream_color_ops(object, reader, builder, object_label);
        }
        PdfObject::Array(items) => {
            for item in items {
                scan_object(item, reader, builder, object_label.clone(), depth + 1);
            }
        }
        _ => {}
    }
}

fn scan_content_stream_color_ops(
    stream: &PdfObject,
    reader: &PdfReader,
    builder: &mut ColorReportBuilder,
    object_label: Option<String>,
) {
    let PdfObject::Stream { dict, .. } = stream else {
        return;
    };
    if matches!(
        dict.get_name("Subtype"),
        Some("Image") | Some("Type1C") | Some("CIDFontType0C")
    ) || dict.get("N").is_some()
    {
        return;
    }
    let Ok(decoded) = decode_stream_lossless(stream, reader) else {
        return;
    };
    if decoded.status != StreamDecodeStatus::Complete {
        return;
    }
    if decoded.data.len() > ColorLimits::default().max_content_stream_color_scan_bytes {
        builder.diagnostic(
            "color.content_stream.scan_cap",
            ColorSeverity::SecurityLimit,
            format!(
                "content stream color scan skipped at {} bytes; cap is {}",
                decoded.data.len(),
                ColorLimits::default().max_content_stream_color_scan_bytes
            ),
            object_label,
        );
        return;
    }
    let Ok(ops) = ContentParser::parse(&decoded.data) else {
        return;
    };
    for op in ops {
        match op.operator.as_str() {
            "G" | "g" => builder.count_space("DeviceGray"),
            "RG" | "rg" => builder.count_space("DeviceRGB"),
            "K" | "k" => builder.count_space("DeviceCMYK"),
            "CS" | "cs" => {
                if let Some(name) = op.name(0) {
                    builder.count_space(name);
                }
            }
            "ri" => {
                if let Some(intent) = op.name(0) {
                    builder.count_intent(intent);
                }
            }
            _ => {}
        }
    }
}

fn scan_dict(
    dict: &PdfDictionary,
    reader: &PdfReader,
    builder: &mut ColorReportBuilder,
    object_label: Option<String>,
    depth: usize,
) {
    if let Some(resources) = dict.get_dict("Resources") {
        scan_dict(resources, reader, builder, object_label.clone(), depth + 1);
    }
    if let Some(color_spaces) = dict.get_dict("ColorSpace") {
        for (_, space) in color_spaces.entries() {
            classify_color_space(space, reader, builder, object_label.clone());
        }
    } else if let Some(space) = dict.get("ColorSpace") {
        classify_color_space(space, reader, builder, object_label.clone());
    }
    if let Some(ext_g_states) = dict.get_dict("ExtGState") {
        for (_, value) in ext_g_states.entries() {
            if let Some(ext) = resolve_to_dict(value, reader) {
                scan_ext_g_state(&ext, builder, object_label.clone());
            }
        }
    }
    scan_ext_g_state(dict, builder, object_label.clone());
    for (key, value) in dict.entries() {
        if matches!(key.as_str(), "ColorSpace" | "ExtGState" | "Resources") {
            continue;
        }
        scan_object(value, reader, builder, object_label.clone(), depth + 1);
    }
}

fn scan_ext_g_state(
    dict: &PdfDictionary,
    builder: &mut ColorReportBuilder,
    object_label: Option<String>,
) {
    let mut saw_ext_state = false;
    if let Some(intent) = dict.get_name("RI") {
        builder.count_intent(intent);
        saw_ext_state = true;
    }
    if dict.get_bool("OP").unwrap_or(false) {
        builder.overprint.stroke_overprint_used = true;
        saw_ext_state = true;
    }
    if dict.get_bool("op").unwrap_or(false) {
        builder.overprint.fill_overprint_used = true;
        saw_ext_state = true;
    }
    if dict.get_integer("OPM").unwrap_or(0) == 1 {
        builder.overprint.overprint_mode_one_used = true;
        saw_ext_state = true;
    }
    if saw_ext_state {
        builder.overprint.ext_gstate_count += 1;
    }
    if dict.get_bool("OP").unwrap_or(false) || dict.get_bool("op").unwrap_or(false) {
        builder.overprint.cmyk_fill_preview_supported = true;
        builder.overprint.approximation_diagnostics += 1;
        builder.diagnostic(
            "color.overprint.preview_approximation",
            ColorSeverity::Info,
            "overprint state is parsed and preserved; DeviceCMYK fills use RGB-framebuffer preview, but true separations compositing is not claimed",
            object_label,
        );
    }
}

fn push_profile_diagnostics(builder: &mut ColorReportBuilder, profile: &prepress::IccProfileInfo) {
    if profile.profile_class == IccProfileClass::Malformed {
        builder.diagnostic(
            "color.icc.malformed_profile_fail_closed",
            ColorSeverity::Warning,
            format!(
                "ICC profile {} is malformed and cannot be used for proofing",
                profile.profile_hash
            ),
            profile.object.clone(),
        );
    }
    if profile.profile_class == IccProfileClass::DeviceLink {
        let severity = if cmm::native_cmm_status().available {
            ColorSeverity::Info
        } else {
            ColorSeverity::Warning
        };
        builder.diagnostic(
            "color.icc.device_link.reported",
            severity,
            format!(
                "device-link ICC profile {} detected; native status: {}; fallback status: {}",
                profile.profile_hash,
                profile.native_transform_status,
                profile.fallback_transform_status
            ),
            profile.object.clone(),
        );
    }
    if profile.is_multicolor {
        builder.diagnostic(
            "color.icc.multicolor.reported",
            ColorSeverity::Warning,
            format!(
                "multicolor ICC profile {} has input channels {:?} and output channels {:?}; unsupported shapes are inventory-only",
                profile.profile_hash, profile.input_channels, profile.output_channels
            ),
            profile.object.clone(),
        );
    }
    if profile.channel_mismatch {
        builder.diagnostic(
            "color.icc.channel_mismatch_fail_closed",
            ColorSeverity::Warning,
            profile
                .reason
                .clone()
                .unwrap_or_else(|| "ICC channel mismatch".to_string()),
            profile.object.clone(),
        );
    }
}

fn classify_color_space(
    space: &PdfObject,
    reader: &PdfReader,
    builder: &mut ColorReportBuilder,
    object_label: Option<String>,
) {
    let resolved = resolve_object(space, reader).unwrap_or_else(|| space.clone());
    match resolved {
        PdfObject::Name(name) => builder.count_space(name),
        PdfObject::Array(arr) => {
            let Some(family) = arr.first().and_then(PdfObject::as_name) else {
                return;
            };
            builder.count_space(family);
            match family {
                "ICCBased" => inspect_icc_space(&arr, reader, builder, object_label),
                "Indexed" => inspect_indexed_space(&arr, reader, builder, object_label),
                "Separation" => inspect_separation_space(&arr, reader, builder, object_label),
                "DeviceN" => inspect_devicen_space(&arr, reader, builder, object_label),
                "CalGray" | "CalRGB" | "Lab" => {}
                _ => {
                    builder.diagnostic(
                        "color.colorspace.unsupported",
                        ColorSeverity::Warning,
                        format!("unsupported color space family {family}"),
                        object_label,
                    );
                }
            }
        }
        _ => {}
    }
}

fn inspect_icc_space(
    arr: &[PdfObject],
    reader: &PdfReader,
    builder: &mut ColorReportBuilder,
    object_label: Option<String>,
) {
    let Some(profile_obj) = arr.get(1) else {
        builder.diagnostic(
            "color.icc.profile_missing",
            ColorSeverity::Error,
            "ICCBased color space lacks profile stream",
            object_label,
        );
        return;
    };
    let Some((dict, stream)) = resolve_stream(profile_obj, reader) else {
        builder.diagnostic(
            "color.icc.profile_missing",
            ColorSeverity::Error,
            "ICCBased profile object is not a stream",
            object_label,
        );
        return;
    };
    if !matches!(dict.get_integer("N"), Some(1 | 3 | 4)) {
        builder.diagnostic(
            "color.icc.unsupported_components",
            ColorSeverity::Warning,
            format!(
                "ICCBased /N {:?} is outside Gray/RGB/CMYK preview scope",
                dict.get_integer("N")
            ),
            object_label.clone(),
        );
    }
    match decode_stream_lossless(&stream, reader) {
        Ok(decoded) if decoded.status == StreamDecodeStatus::Complete => {
            if decoded.data.len() > cmm::DEFAULT_MAX_ICC_PROFILE_BYTES {
                builder.diagnostic(
                    "color.icc.profile_too_large",
                    ColorSeverity::SecurityLimit,
                    format!(
                        "ICCBased profile is {} bytes; cap is {}",
                        decoded.data.len(),
                        cmm::DEFAULT_MAX_ICC_PROFILE_BYTES
                    ),
                    object_label,
                );
            } else {
                let profile_info = prepress::classify_icc_profile(
                    &decoded.data,
                    dict.get_integer("N").and_then(|n| u8::try_from(n).ok()),
                    object_label.clone(),
                );
                push_profile_diagnostics(builder, &profile_info);
                builder.icc_profiles.push(profile_info);
                if qcms::Profile::new_from_slice(&decoded.data, false).is_none() {
                    builder.diagnostic(
                        "color.icc.invalid_profile",
                        ColorSeverity::Warning,
                        "ICCBased profile could not be parsed by qcms; alternate/fallback path will be used",
                        object_label,
                    );
                }
            }
        }
        _ => builder.diagnostic(
            "color.icc.decode_failed",
            ColorSeverity::Warning,
            "ICCBased profile stream could not be losslessly decoded",
            object_label,
        ),
    }
}

fn inspect_indexed_space(
    arr: &[PdfObject],
    reader: &PdfReader,
    builder: &mut ColorReportBuilder,
    object_label: Option<String>,
) {
    if let Some(base) = arr.get(1) {
        classify_color_space(base, reader, builder, object_label.clone());
    }
    let hival = arr.get(2).and_then(PdfObject::as_integer).unwrap_or(-1);
    if !(0..=255).contains(&hival) {
        builder.diagnostic(
            "color.indexed.invalid_hival",
            ColorSeverity::Warning,
            format!("Indexed hival {hival} is outside 0..255"),
            object_label.clone(),
        );
    }
    let Some(lookup) = arr.get(3) else {
        builder.diagnostic(
            "color.indexed.lookup_missing",
            ColorSeverity::Error,
            "Indexed color space lacks lookup table",
            object_label,
        );
        return;
    };
    let bytes = match lookup {
        PdfObject::String(bytes) => Some(bytes.len()),
        PdfObject::Stream { raw, .. } => Some(raw.len()),
        PdfObject::Reference { .. } => resolve_object(lookup, reader).and_then(|obj| match obj {
            PdfObject::String(bytes) => Some(bytes.len()),
            PdfObject::Stream { raw, .. } => Some(raw.len()),
            _ => None,
        }),
        _ => None,
    };
    if bytes.is_none() {
        builder.diagnostic(
            "color.indexed.lookup_malformed",
            ColorSeverity::Warning,
            "Indexed lookup table is neither a string nor stream",
            object_label,
        );
    }
}

fn inspect_separation_space(
    arr: &[PdfObject],
    reader: &PdfReader,
    builder: &mut ColorReportBuilder,
    object_label: Option<String>,
) {
    builder.spot_preview.separation_spaces += 1;
    let space_obj = PdfObject::Array(arr.to_vec());
    let contributions = prepress::plate_contributions_for_color_space(
        &space_obj,
        &[1.0],
        1.0,
        reader,
        object_label.clone(),
        "color_space_inventory",
        None,
    );
    builder.separation_framebuffer.record_all(contributions);
    if let Some(name) = arr.get(1).and_then(PdfObject::as_name) {
        builder.spot_colorants.insert(name.to_string());
    }
    if let Some(alt) = arr.get(2) {
        classify_color_space(alt, reader, builder, object_label.clone());
    }
    if arr.get(3).is_some() {
        builder.spot_preview.separation_tint_transforms += 1;
    } else {
        builder.spot_preview.missing_tint_transforms += 1;
        builder.diagnostic(
            "color.separation.tint_transform_missing",
            ColorSeverity::Error,
            "Separation color space lacks tint transform",
            object_label,
        );
    }
}

fn inspect_devicen_space(
    arr: &[PdfObject],
    reader: &PdfReader,
    builder: &mut ColorReportBuilder,
    object_label: Option<String>,
) {
    builder.spot_preview.devicen_spaces += 1;
    let names: Vec<String> = arr
        .get(1)
        .and_then(PdfObject::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(PdfObject::as_name)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if !names.is_empty() {
        builder.devicen_components.insert(names.clone());
    }
    if names.len() > colorspace::MAX_DEVICEN_COMPONENTS {
        builder.diagnostic(
            "color.devicen.component_cap",
            ColorSeverity::SecurityLimit,
            format!(
                "DeviceN has {} components; cap is {}",
                names.len(),
                colorspace::MAX_DEVICEN_COMPONENTS
            ),
            object_label.clone(),
        );
    }
    let default_tints = vec![1.0; names.len().max(1)];
    let space_obj = PdfObject::Array(arr.to_vec());
    let contributions = prepress::plate_contributions_for_color_space(
        &space_obj,
        &default_tints,
        1.0,
        reader,
        object_label.clone(),
        "color_space_inventory",
        None,
    );
    builder.separation_framebuffer.record_all(contributions);
    if let Some(alt) = arr.get(2) {
        classify_color_space(alt, reader, builder, object_label.clone());
    }
    if arr.get(3).is_some() {
        builder.spot_preview.devicen_tint_transforms += 1;
    } else {
        builder.spot_preview.missing_tint_transforms += 1;
        builder.diagnostic(
            "color.devicen.tint_transform_missing",
            ColorSeverity::Error,
            "DeviceN color space lacks tint transform",
            object_label,
        );
    }
}

fn resolve_object(obj: &PdfObject, reader: &PdfReader) -> Option<PdfObject> {
    match obj {
        PdfObject::Reference { .. } => reader.resolve(obj.clone()).ok(),
        other => Some(other.clone()),
    }
}

fn resolve_to_dict(obj: &PdfObject, reader: &PdfReader) -> Option<PdfDictionary> {
    match resolve_object(obj, reader)? {
        PdfObject::Dictionary(dict) => Some(dict),
        PdfObject::Stream { dict, .. } => Some(dict),
        _ => None,
    }
}

fn resolve_stream(obj: &PdfObject, reader: &PdfReader) -> Option<(PdfDictionary, PdfObject)> {
    match resolve_object(obj, reader)? {
        PdfObject::Stream { dict, raw } => {
            let profile_dict = dict.clone();
            Some((profile_dict, PdfObject::Stream { dict, raw }))
        }
        _ => None,
    }
}

fn as_pdf_string(obj: &PdfObject) -> Option<String> {
    match obj {
        PdfObject::String(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
        PdfObject::Name(name) => Some(name.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_pdf(objects: &[&str], trailer_root: &str) -> Vec<u8> {
        let mut pdf = Vec::new();
        pdf.extend_from_slice(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n");
        let mut offsets = vec![0usize];
        for object in objects {
            offsets.push(pdf.len());
            pdf.extend_from_slice(object.as_bytes());
            pdf.extend_from_slice(b"\n");
        }
        let xref = pdf.len();
        pdf.extend_from_slice(format!("xref\n0 {}\n", offsets.len()).as_bytes());
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        for offset in offsets.iter().skip(1) {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root {trailer_root} >>\nstartxref\n{xref}\n%%EOF\n",
                offsets.len()
            )
            .as_bytes(),
        );
        pdf
    }

    #[test]
    fn reports_output_intent_and_invalid_icc_profile() {
        let pdf = build_pdf(
            &[
                "1 0 obj\n<< /Type /Catalog /Pages 2 0 R /OutputIntents [3 0 R] >>\nendobj",
                "2 0 obj\n<< /Type /Pages /Count 0 >>\nendobj",
                "3 0 obj\n<< /S /GTS_PDFA1 /OutputConditionIdentifier (sRGB) /DestOutputProfile 4 0 R >>\nendobj",
                "4 0 obj\n<< /N 3 /Length 4 >>\nstream\nbad!\nendstream\nendobj",
            ],
            "1 0 R",
        );
        let report = color_report_bytes(&pdf, ColorValidationProfile::PdfA).unwrap();
        assert_eq!(report.output_intents.len(), 1);
        assert!(report.standards.output_intent_checked);
        assert!(report.standards.icc_profile_checked);
        assert!(report.icc_fidelity_vectors.iter().all(|v| v.passed));
        assert!(report.output_intents[0].dest_output_profile_present);
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.code == "color.icc.invalid_profile"));
    }

    #[test]
    fn pdfa_profile_requires_output_intent() {
        let pdf = build_pdf(
            &[
                "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj",
                "2 0 obj\n<< /Type /Pages /Count 0 >>\nendobj",
            ],
            "1 0 R",
        );
        let report = color_report_bytes(&pdf, ColorValidationProfile::PdfA).unwrap();
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.code == "color.output_intent.missing"));
    }

    #[test]
    fn reports_spot_devicen_overprint_and_intent() {
        let pdf = build_pdf(
            &[
                "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj",
                "2 0 obj\n<< /Type /Pages /Count 1 /Kids [5 0 R] >>\nendobj",
                "3 0 obj\n<< /FunctionType 2 /Domain [0 1] /Range [0 1 0 1 0 1] /C0 [0 0 0] /C1 [1 0 0] /N 1 >>\nendobj",
                "4 0 obj\n<< /OP true /op true /OPM 1 /RI /Perceptual >>\nendobj",
                "5 0 obj\n<< /Type /Page /Parent 2 0 R /Resources << /ColorSpace << /CS1 [/Separation /SpotBlue /DeviceRGB 3 0 R] /CS2 [/DeviceN [/Cyan /Spot] /DeviceCMYK 3 0 R] >> /ExtGState << /GS1 4 0 R >> >> >>\nendobj",
            ],
            "1 0 R",
        );
        let report = color_report_bytes(&pdf, ColorValidationProfile::Generic).unwrap();
        assert!(report.spot_colorants.iter().any(|s| s == "SpotBlue"));
        assert!(report.spot_preview.separation_spaces >= 1);
        assert!(report.spot_preview.devicen_spaces >= 1);
        assert!(report.spot_preview.separation_tint_transforms >= 1);
        assert!(report.spot_preview.devicen_tint_transforms >= 1);
        assert!(report
            .devicen_components
            .iter()
            .any(|names| names == &vec!["Cyan".to_string(), "Spot".to_string()]));
        assert!(report.overprint.stroke_overprint_used);
        assert!(report.overprint.fill_overprint_used);
        assert!(report.overprint.overprint_mode_one_used);
        assert!(report.overprint.cmyk_fill_preview_supported);
        assert!(report.overprint.true_separation_framebuffer);
        assert!(report.overprint.approximation_diagnostics >= 1);
        assert_eq!(
            report
                .prompt12_prepress_cmm_device_link_separation_plates
                .separation_framebuffer
                .plate_count,
            3
        );
        assert!(report
            .prompt12_prepress_cmm_device_link_separation_plates
            .spot_plates
            .iter()
            .any(|plate| plate.plane_name == "SpotBlue"));
        assert!(report
            .rendering_intents
            .iter()
            .any(|intent| intent.family == "Perceptual"));
    }

    #[test]
    fn reports_device_cmyk_from_content_stream_operator() {
        let content = "0.1 0.2 0.3 0.4 k\n0.2 0.1 0.0 0.0 K\n";
        let pdf = build_pdf(
            &[
                "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj",
                "2 0 obj\n<< /Type /Pages /Count 1 /Kids [3 0 R] >>\nendobj",
                "3 0 obj\n<< /Type /Page /Parent 2 0 R /Resources << >> /Contents 4 0 R >>\nendobj",
                &format!(
                    "4 0 obj\n<< /Length {} >>\nstream\n{content}endstream\nendobj",
                    content.len()
                ),
            ],
            "1 0 R",
        );
        let report = color_report_bytes(&pdf, ColorValidationProfile::Generic).unwrap();
        let cmyk = report
            .color_spaces
            .iter()
            .find(|usage| usage.family == "DeviceCMYK")
            .expect("DeviceCMYK usage");
        assert_eq!(cmyk.count, 2);
    }

    #[test]
    fn diagnoses_devicen_component_cap() {
        let names = (0..20)
            .map(|i| format!("/C{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let pdf = build_pdf(
            &[
                "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj",
                "2 0 obj\n<< /Type /Pages /Count 1 /Kids [4 0 R] >>\nendobj",
                "3 0 obj\n<< /FunctionType 2 /Domain [0 1] /Range [0 1 0 1 0 1] /C0 [0 0 0] /C1 [1 1 1] /N 1 >>\nendobj",
                &format!("4 0 obj\n<< /Type /Page /Parent 2 0 R /Resources << /ColorSpace << /CS1 [/DeviceN [{names}] /DeviceRGB 3 0 R] >> >> >>\nendobj"),
            ],
            "1 0 R",
        );
        let report = color_report_bytes(&pdf, ColorValidationProfile::Generic).unwrap();
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.code == "color.devicen.component_cap"));
    }

    #[test]
    fn pdfx_profile_checks_output_intent_s_and_cmyk_profile() {
        let pdf = build_pdf(
            &[
                "1 0 obj\n<< /Type /Catalog /Pages 2 0 R /OutputIntents [3 0 R] >>\nendobj",
                "2 0 obj\n<< /Type /Pages /Count 0 >>\nendobj",
                "3 0 obj\n<< /S /GTS_PDFA1 /OutputConditionIdentifier (sRGB) /DestOutputProfile 4 0 R >>\nendobj",
                "4 0 obj\n<< /N 3 /Length 4 >>\nstream\nbad!\nendstream\nendobj",
            ],
            "1 0 R",
        );
        let report = color_report_bytes(&pdf, ColorValidationProfile::PdfX).unwrap();
        assert!(report.standards.output_intent_checked);
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.code == "color.pdfx.output_intent.s"));
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.code == "color.pdfx.output_profile.not_cmyk"));
    }
}
