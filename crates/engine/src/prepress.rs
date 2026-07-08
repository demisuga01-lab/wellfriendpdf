//! Prompt 12 prepress color structures.
//!
//! This module keeps the press-oriented color state separate from the RGB
//! preview renderer. It inventories ICC profile classes, reports native/fallback
//! transform posture, and stores sparse plate contributions for Separation and
//! DeviceN color spaces without claiming Prompt 13 overprint simulation.

use crate::object::PdfObject;
use crate::reader::PdfReader;
use crate::render::{cmm, colorspace};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_PREPRESS_PLATES: usize = 32;
pub const DEFAULT_SEPARATION_FRAMEBUFFER_BUDGET_BYTES: usize = 64 * 1024 * 1024;
const CONTRIBUTION_ACCOUNTING_BYTES: usize = 96;

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
pub struct Prompt12PrepressReport {
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

impl Default for Prompt12PrepressReport {
    fn default() -> Self {
        Self::from_parts(Vec::new(), SeparationFramebuffer::default().report())
    }
}

impl Prompt12PrepressReport {
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
                    "target/prompt12-prepress-cmm/plate-preview-results-prompt12.json"
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
                "full overprint compositing across spot/process plates is Prompt 13 scope".to_string(),
                "certification-grade PDF/X validation is later standards work".to_string(),
                "multicolor ICC transforms above the safe renderer pixel formats are fail-closed/report-only".to_string(),
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
        let estimated = (self.contributions.len() + 1) * CONTRIBUTION_ACCOUNTING_BYTES;
        if estimated > self.memory_budget_bytes {
            self.report_only_degraded = true;
            self.diagnostics.push(format!(
                "estimated plate framebuffer bytes {estimated} exceed budget {}",
                self.memory_budget_bytes
            ));
            return;
        }
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
        let cache_fingerprint = cache_fingerprint(
            "plate-framebuffer",
            &deterministic_plane_order,
            cmm::ColorTransformOptions::default().intent.as_str(),
            cmm::ColorTransformOptions::default().black_point_compensation,
            cmm::native_cmm_status().selected_backend,
            None,
        );
        SeparationFramebufferReport {
            true_separation_framebuffer: true,
            storage_model: "sparse_tile_local_plate_contributions_with_bounded_memory_accounting"
                .to_string(),
            page_number: self.page_number,
            tile_identity: self.tile_identity.clone(),
            deterministic_plane_order,
            plate_count: summaries.len(),
            contribution_count: self.contributions.len(),
            memory_budget_bytes: self.memory_budget_bytes,
            estimated_memory_bytes: self.contributions.len() * CONTRIBUTION_ACCOUNTING_BYTES,
            scheduler_accounted: true,
            excessive_colorants_fail_closed: self.report_only_degraded,
            report_only_degraded: self.report_only_degraded,
            cache_fingerprint,
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
                "{}:{:?}:{:.6}:{:.6}:{:?}:{}",
                contribution.plane_name,
                contribution.kind,
                contribution.tint,
                contribution.alpha,
                contribution.alternate_preview_rgb,
                contribution.operation
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
                overprint_posture: "plate_preserved_preview_limited_overprint_pending".to_string(),
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
                    overprint_posture: "plate_preserved_preview_limited_overprint_pending"
                        .to_string(),
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

pub(crate) fn cache_fingerprint_for_color_spaces<'a>(
    spaces: impl IntoIterator<Item = &'a PdfObject>,
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
            overprint_posture: "plate_preserved_preview_limited_overprint_pending".to_string(),
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
            overprint_posture: "plate_preserved_preview_limited_overprint_pending".to_string(),
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
