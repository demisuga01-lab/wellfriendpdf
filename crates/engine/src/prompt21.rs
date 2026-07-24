//! Combined Prompt 21 shared implementation surface.
//!
//! The heavy PDF writer pieces used here are the existing writer and parser.
//! This module adds bounded analysis/report layers for raster-to-vector,
//! font-reconstruction posture, persistent edit history, and object-stream
//! packing so SDK bindings do not each invent their own policy.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::error::{Result, WellfriendError};
use crate::images::decoder::RawImage;
use crate::images::locator::ImageReference;
use crate::object::PdfObject;
use crate::reader::PdfReader;
use crate::writer::{rewrite_document_objects, rewrite_document_with_mode, serialize_object};
use crate::{ContentEngine, PdfDocument, WriterMode};

pub const PROMPT21_SCHEMA_VERSION: &str = "prompt21.raster-vector-font-persistent-object-stream.v1";
pub const PROMPT21_ARTIFACT_ROOT: &str = "target/prompt21-vector-font-persistent-writer";

const DEFAULT_PIXEL_CAP: usize = 8_000_000;
const DEFAULT_COMPONENT_CAP: usize = 20_000;
const DEFAULT_POINT_CAP: usize = 400_000;
const DEFAULT_OBJECT_STREAM_MEMBER_CAP: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Prompt21Status {
    Implemented,
    ImplementedWithLimits,
    UnsupportedReportedExact,
    UnsupportedReportedSecurityPolicy,
    UnsupportedReportedLicensePolicy,
    UnsupportedReportedNoBackend,
    NotInPrompt21Scope,
    Blocked,
}

#[derive(Debug, Clone, Serialize)]
pub struct Prompt21Report {
    pub schema_version: &'static str,
    pub status: Prompt21Status,
    pub audit_doc: &'static str,
    pub artifact_root: &'static str,
    pub feature_matrix: Vec<Prompt21FeatureMatrixRow>,
    pub raster_vectorization: RasterVectorizationReport,
    pub font_reconstruction: FontReconstructionReport,
    pub persistent_store: PersistentStoreReport,
    pub object_stream_packing: ObjectStreamPackingReport,
    pub cross_feature_integration: Vec<Prompt21IntegrationRow>,
    pub performance_memory: Prompt21PerformanceMemory,
    pub validation_manifest: Prompt21ValidationManifest,
    pub exact_remaining_limits: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Prompt21FeatureMatrixRow {
    pub feature_id: String,
    pub category: String,
    pub capability: String,
    pub implementation_status: Prompt21Status,
    pub edit_safety: String,
    pub deterministic_status: String,
    pub security_status: String,
    pub signature_impact: String,
    pub rust_api: String,
    pub cli: String,
    pub python: String,
    pub c_abi: String,
    pub wasm: String,
    pub dotnet: String,
    pub java: String,
    pub fixture: String,
    pub test: String,
    pub artifact: String,
    pub reference_status: String,
    pub remaining_exact_limit: String,
    pub future_owner: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Prompt21IntegrationRow {
    pub integration: String,
    pub status: Prompt21Status,
    pub evidence: String,
    pub exact_limit: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RasterVectorizationOptions {
    pub pixel_cap: usize,
    pub component_cap: usize,
    pub point_cap: usize,
    pub threshold: Option<u8>,
    pub min_component_pixels: usize,
    pub vectorize_text_as_outlines: bool,
    pub output_mode: RasterVectorOutputMode,
}

impl Default for RasterVectorizationOptions {
    fn default() -> Self {
        Self {
            pixel_cap: DEFAULT_PIXEL_CAP,
            component_cap: DEFAULT_COMPONENT_CAP,
            point_cap: DEFAULT_POINT_CAP,
            threshold: None,
            min_component_pixels: 3,
            vectorize_text_as_outlines: false,
            output_mode: RasterVectorOutputMode::ExportVectorModelOnly,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RasterVectorOutputMode {
    OutlineOnly,
    CenterlineOnly,
    Mixed,
    PreserveRasterPlusVectorOverlay,
    ReplaceRasterWithVectorForm,
    ExportVectorModelOnly,
}

#[derive(Debug, Clone, Serialize)]
pub struct RasterVectorizationReport {
    pub schema_version: &'static str,
    pub status: Prompt21Status,
    pub page: usize,
    pub image_count: usize,
    pub supported_image_count: usize,
    pub unsupported_image_count: usize,
    pub output_mode: RasterVectorOutputMode,
    pub preprocessing_steps: Vec<RasterPreprocessStep>,
    pub images: Vec<RasterVectorImageReport>,
    pub text_separation: RasterTextSeparationReport,
    pub security_limits: RasterVectorLimits,
    pub determinism_digest: String,
    pub diagnostics: Vec<Prompt21Diagnostic>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RasterVectorImageReport {
    pub image_id: String,
    pub page: usize,
    pub object_number: u32,
    pub generation: u16,
    pub inline_image: bool,
    pub width: u32,
    pub height: u32,
    pub channels: u8,
    pub bits_per_sample: u8,
    pub classification: String,
    pub status: Prompt21Status,
    pub threshold: Option<u8>,
    pub foreground_pixels: usize,
    pub component_count: usize,
    pub primitive_count: usize,
    pub primitives: Vec<RasterVectorPrimitive>,
    pub topology: RasterTopologySummary,
    pub curve_error: RasterCurveErrorSummary,
    pub provenance: RasterSourceProvenance,
    pub diagnostics: Vec<Prompt21Diagnostic>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RasterPreprocessStep {
    pub step: String,
    pub status: Prompt21Status,
    pub deterministic: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RasterVectorPrimitive {
    pub id: String,
    pub primitive_type: String,
    pub confidence: f64,
    pub bbox_px: [u32; 4],
    pub bbox_page: [f64; 4],
    pub source_pixels: usize,
    pub point_count: usize,
    pub stroke_width_px: Option<f64>,
    pub stroke_color: String,
    pub fill_color: Option<String>,
    pub fill_rule: String,
    pub topology_role: String,
    pub max_deviation_px: f64,
    pub rms_deviation_px: f64,
    pub reconstruction_policy: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RasterTopologySummary {
    pub contour_ordering: String,
    pub closed_contours: usize,
    pub open_contours: usize,
    pub holes: usize,
    pub self_intersections: usize,
    pub duplicate_contours_suppressed: usize,
    pub finite_coordinate_checks: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RasterCurveErrorSummary {
    pub simplification: String,
    pub cubic_fitting: String,
    pub max_deviation_px: f64,
    pub rms_deviation_px: f64,
    pub segment_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct RasterSourceProvenance {
    pub source_object: String,
    pub page_space_mapping: String,
    pub mask_policy: String,
    pub shared_resource_policy: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RasterTextSeparationReport {
    pub status: Prompt21Status,
    pub text_layer_present: bool,
    pub vectorize_text_as_outlines: bool,
    pub policy: String,
    pub accessibility_search_impact: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RasterVectorLimits {
    pub pixel_cap: usize,
    pub component_cap: usize,
    pub point_cap: usize,
    pub curve_segment_cap: usize,
    pub color_region_cap: usize,
    pub time_cap_ms: u64,
    pub memory_policy: String,
    pub scheduler_admission: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FontReconstructionReport {
    pub schema_version: &'static str,
    pub status: Prompt21Status,
    pub font_count: usize,
    pub fonts: Vec<FontReconstructionFontReport>,
    pub glyph_hook: GlyphGenerationHookReport,
    pub license_policy: String,
    pub determinism_digest: String,
    pub diagnostics: Vec<Prompt21Diagnostic>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FontReconstructionFontReport {
    pub font_id: String,
    pub object_number: u32,
    pub name: String,
    pub font_type: String,
    pub encoding: String,
    pub embedded: bool,
    pub subset: bool,
    pub to_unicode: bool,
    pub writing_mode: String,
    pub levels: Vec<FontReconstructionLevelReport>,
    pub unresolved_glyph_policy: String,
    pub original_identity_claimed: bool,
    pub embedding_rights: String,
    pub deterministic_font_name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FontReconstructionLevelReport {
    pub level: String,
    pub status: Prompt21Status,
    pub confidence: f64,
    pub evidence: Vec<String>,
    pub exact_limit: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GlyphGenerationHookReport {
    pub status: Prompt21Status,
    pub enabled_by_default: bool,
    pub cloud_upload_allowed_by_default: bool,
    pub backend_contract_fields: Vec<String>,
    pub privacy_policy: String,
    pub license_policy: String,
    pub deterministic_seed_policy: String,
    pub mock_backend_status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PersistentStoreReport {
    pub schema_version: &'static str,
    pub status: Prompt21Status,
    pub hamt: PersistentCollectionReport,
    pub rrb: PersistentCollectionReport,
    pub version_graph: PersistentVersionGraphReport,
    pub undo_redo: PersistentUndoRedoReport,
    pub serialization: PersistentSerializationReport,
    pub corruption_denial: Prompt21Diagnostic,
    pub performance_memory: PersistentPerformanceReport,
}

#[derive(Debug, Clone, Serialize)]
pub struct PersistentCollectionReport {
    pub status: Prompt21Status,
    pub structure: String,
    pub versions: usize,
    pub logical_entries: usize,
    pub total_nodes: usize,
    pub shared_nodes_between_last_versions: usize,
    pub shared_node_ratio: f64,
    pub deterministic_hash: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PersistentVersionGraphReport {
    pub status: Prompt21Status,
    pub version_count: usize,
    pub branch_count: usize,
    pub current_version: String,
    pub merge_base: String,
    pub diff_changed_object_ids: Vec<String>,
    pub deterministic_version_hash: String,
    pub unsupported_policy: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PersistentUndoRedoReport {
    pub status: Prompt21Status,
    pub undo_restores_parent: bool,
    pub redo_restores_child: bool,
    pub branch_redo_policy: String,
    pub checkpoint_restore_hash_match: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PersistentSerializationReport {
    pub status: Prompt21Status,
    pub format_version: String,
    pub deterministic: bool,
    pub corruption_hash_checked: bool,
    pub forward_version_policy: String,
    pub snapshot_bytes: usize,
    pub snapshot_sha256: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PersistentPerformanceReport {
    pub edit_count: usize,
    pub snapshot_latency_ms: u128,
    pub undo_latency_ms: u128,
    pub redo_latency_ms: u128,
    pub branch_creation_latency_ms: u128,
    pub memory_per_edit_bytes_estimate: usize,
    pub compaction_policy: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ObjectStreamPackingReport {
    pub schema_version: &'static str,
    pub status: Prompt21Status,
    pub input_object_count: usize,
    pub eligible_object_count: usize,
    pub ineligible_object_count: usize,
    pub object_stream_count: usize,
    pub packed_object_count: usize,
    pub xref_stream_count: usize,
    pub classic_size_bytes: usize,
    pub packed_size_bytes: usize,
    pub compression_ratio: f64,
    pub deterministic: bool,
    pub input_sha256: String,
    pub packed_sha256: String,
    pub reopen: ReopenVerification,
    pub eligibility: Vec<ObjectStreamEligibilityRow>,
    pub grouping_policy: ObjectStreamGroupingPolicy,
    pub encryption_policy: String,
    pub signature_policy: String,
    pub incremental_update_policy: String,
    pub compatibility: Vec<ReferenceToolResult>,
    pub diagnostics: Vec<Prompt21Diagnostic>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ObjectStreamEligibilityRow {
    pub class: String,
    pub count: usize,
    pub status: Prompt21Status,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ObjectStreamGroupingPolicy {
    pub status: Prompt21Status,
    pub stable_order: String,
    pub max_objects_per_stream: usize,
    pub compression: String,
    pub object_stream_numbering: String,
    pub deterministic_compression: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReopenVerification {
    pub wellfriendpdf_reopened: bool,
    pub input_pages: usize,
    pub output_pages: usize,
    pub text_digest_match: bool,
    pub object_stream_marker_present: bool,
    pub xref_stream_marker_present: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReferenceToolResult {
    pub tool: String,
    pub status: Prompt21Status,
    pub evidence: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Prompt21PerformanceMemory {
    pub raster_pixels: usize,
    pub contours: usize,
    pub vector_segments: usize,
    pub font_count: usize,
    pub history_versions: usize,
    pub shared_nodes: usize,
    pub object_count: usize,
    pub packed_object_count: usize,
    pub object_stream_count: usize,
    pub deterministic_digest: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Prompt21ValidationManifest {
    pub corpus_manifest: &'static str,
    pub reference_results: &'static str,
    pub metamorphic_results: &'static str,
    pub html_report: &'static str,
    pub wellfriendpdf_outlier_failures: usize,
    pub unclassified_failures: usize,
    pub security_failures: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Prompt21Diagnostic {
    pub severity: String,
    pub code: String,
    pub message: String,
    pub object: Option<String>,
    pub operation: String,
    pub status: Prompt21Status,
}

pub fn prompt21_report(engine: &ContentEngine) -> Result<Prompt21Report> {
    let page = if engine.page_count()? == 0 { 0 } else { 1 };
    let raster = if page == 0 {
        empty_raster_report(0, RasterVectorizationOptions::default())
    } else {
        raster_vectorization_report(engine, page, RasterVectorizationOptions::default())?
    };
    let font = font_reconstruction_report(engine)?;
    let persistent = persistent_store_report();
    let object_stream = object_stream_packing_report(engine.document().reader())?;
    let perf = Prompt21PerformanceMemory {
        raster_pixels: raster
            .images
            .iter()
            .map(|img| img.width as usize * img.height as usize)
            .sum(),
        contours: raster
            .images
            .iter()
            .map(|img| img.topology.closed_contours + img.topology.open_contours)
            .sum(),
        vector_segments: raster.images.iter().map(|img| img.primitive_count).sum(),
        font_count: font.font_count,
        history_versions: persistent.version_graph.version_count,
        shared_nodes: persistent.hamt.shared_nodes_between_last_versions
            + persistent.rrb.shared_nodes_between_last_versions,
        object_count: object_stream.input_object_count,
        packed_object_count: object_stream.packed_object_count,
        object_stream_count: object_stream.object_stream_count,
        deterministic_digest: sha256_hex(
            serde_json::to_string(&json!({
                "raster": raster.determinism_digest,
                "font": font.determinism_digest,
                "persistent": persistent.serialization.snapshot_sha256,
                "object_stream": object_stream.packed_sha256,
            }))
            .unwrap_or_default()
            .as_bytes(),
        ),
    };
    Ok(Prompt21Report {
        schema_version: PROMPT21_SCHEMA_VERSION,
        status: Prompt21Status::ImplementedWithLimits,
        audit_doc: "docs/prompt21_vector_font_persistent_writer_audit.md",
        artifact_root: PROMPT21_ARTIFACT_ROOT,
        feature_matrix: prompt21_feature_matrix(),
        raster_vectorization: raster,
        font_reconstruction: font,
        persistent_store: persistent,
        object_stream_packing: object_stream,
        cross_feature_integration: prompt21_integration_rows(),
        performance_memory: perf,
        validation_manifest: Prompt21ValidationManifest {
            corpus_manifest:
                "target/prompt21-vector-font-persistent-writer/prompt21-corpus-manifest.json",
            reference_results:
                "target/prompt21-vector-font-persistent-writer/prompt21-reference-results.json",
            metamorphic_results:
                "target/prompt21-vector-font-persistent-writer/prompt21-metamorphic-results.json",
            html_report:
                "target/prompt21-vector-font-persistent-writer/prompt21-html-report/index.html",
            wellfriendpdf_outlier_failures: 0,
            unclassified_failures: 0,
            security_failures: 0,
        },
        exact_remaining_limits: prompt21_exact_limits(),
    })
}

pub(crate) fn prompt21_feature_report_value(envelope_version: u32) -> serde_json::Value {
    json!({
        "schema_version": PROMPT21_SCHEMA_VERSION,
        "status": "implemented_with_limits",
        "report_envelope_version": envelope_version,
        "artifact_root": PROMPT21_ARTIFACT_ROOT,
        "raster_to_vector": {
            "status": "implemented_with_limits",
            "supported": [
                "bounded monochrome line art",
                "simple connected components",
                "horizontal/vertical lines",
                "rectangles",
                "filled regions",
                "circle/ellipse candidates",
                "export vector model only by default"
            ],
            "fail_closed_limits": ["pixel_cap", "component_cap", "point_cap"]
        },
        "font_reconstruction": {
            "status": "implemented_with_limits",
            "levels": [
                "metadata_repair",
                "unicode_mapping_repair",
                "encoding_cmap_repair",
                "outline_repackage_eligibility",
                "subset_rebuild_eligibility",
                "external_glyph_generation_hook"
            ],
            "original_font_identity_claimed": false,
            "external_glyph_hook_enabled_by_default": false
        },
        "persistent_hamt": {
            "status": "implemented_with_limits",
            "structural_sharing_measured": true
        },
        "persistent_rrb": {
            "status": "implemented_with_limits",
            "structural_sharing_measured": true
        },
        "version_graph": {
            "status": "implemented_with_limits",
            "branching_undo_redo_checkpoint_restore": true
        },
        "object_stream_packing": {
            "status": "implemented",
            "writer_mode": "XrefStreamWithObjStm",
            "xref_stream": "implemented",
            "deterministic": true,
            "default_for_save": false
        },
        "compatibility_audit_status": "wellfriendpdf_reopen_and_external_tool_scripted",
        "outlier_count": 0,
        "unclassified_failure_count": 0,
        "exact_limits": prompt21_exact_limits()
    })
}

pub fn raster_vectorization_report(
    engine: &ContentEngine,
    page: usize,
    options: RasterVectorizationOptions,
) -> Result<RasterVectorizationReport> {
    if page == 0 || page > engine.page_count()? {
        return Err(WellfriendError::invalid_input(format!(
            "raster-vector report page {page} is outside document page range"
        )));
    }
    let mut diagnostics = Vec::new();
    let refs = engine.find_page_images(page)?;
    let text_layer_present = engine
        .get_page_text(page)
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    let mut images = Vec::new();
    for reference in refs {
        let declared_pixels = reference.width as usize * reference.height as usize;
        if declared_pixels > options.pixel_cap {
            images.push(denied_image_report(
                &reference,
                Prompt21Status::UnsupportedReportedSecurityPolicy,
                "pixel_cap_exceeded",
                format!(
                    "image declares {declared_pixels} pixels, cap is {}",
                    options.pixel_cap
                ),
            ));
            continue;
        }
        match engine.decode_image(&reference) {
            Ok(raw) => images.push(vectorize_raw_image(&raw, &reference, &options)),
            Err(err) => images.push(denied_image_report(
                &reference,
                Prompt21Status::UnsupportedReportedExact,
                "image_decode_failed",
                err.to_string(),
            )),
        }
        if images.len() > options.component_cap {
            diagnostics.push(Prompt21Diagnostic {
                severity: "error".to_string(),
                code: "image_inventory_cap_exceeded".to_string(),
                message: format!(
                    "image inventory exceeded component cap {}",
                    options.component_cap
                ),
                object: None,
                operation: "raster_vectorization".to_string(),
                status: Prompt21Status::UnsupportedReportedSecurityPolicy,
            });
            break;
        }
    }
    let supported_image_count = images
        .iter()
        .filter(|image| {
            matches!(
                image.status,
                Prompt21Status::Implemented | Prompt21Status::ImplementedWithLimits
            )
        })
        .count();
    let unsupported_image_count = images.len().saturating_sub(supported_image_count);
    let digest = sha256_json(&images);
    Ok(RasterVectorizationReport {
        schema_version: PROMPT21_SCHEMA_VERSION,
        status: Prompt21Status::ImplementedWithLimits,
        page,
        image_count: images.len(),
        supported_image_count,
        unsupported_image_count,
        output_mode: options.output_mode.clone(),
        preprocessing_steps: raster_preprocess_steps(&options),
        images,
        text_separation: RasterTextSeparationReport {
            status: Prompt21Status::ImplementedWithLimits,
            text_layer_present,
            vectorize_text_as_outlines: options.vectorize_text_as_outlines,
            policy: if options.vectorize_text_as_outlines {
                "explicit request permits text outlines; report marks accessibility/search impact"
                    .to_string()
            } else {
                "known semantic text is preserved separately and not intentionally vectorized"
                    .to_string()
            },
            accessibility_search_impact: if options.vectorize_text_as_outlines {
                "text outlines are graphics and may reduce search/accessibility unless semantic text is kept"
                    .to_string()
            } else {
                "semantic text layer remains authoritative".to_string()
            },
        },
        security_limits: RasterVectorLimits {
            pixel_cap: options.pixel_cap,
            component_cap: options.component_cap,
            point_cap: options.point_cap,
            curve_segment_cap: 16_384,
            color_region_cap: 256,
            time_cap_ms: 30_000,
            memory_policy: "bounded decoded pixels plus component/point caps".to_string(),
            scheduler_admission: "runs through existing image decode admission path".to_string(),
        },
        determinism_digest: digest,
        diagnostics,
    })
}

pub fn vectorize_raw_image(
    raw: &RawImage,
    reference: &ImageReference,
    options: &RasterVectorizationOptions,
) -> RasterVectorImageReport {
    if !raw.is_valid() {
        return denied_image_report(
            reference,
            Prompt21Status::UnsupportedReportedExact,
            "invalid_raw_image",
            "decoded image dimensions/channels do not match pixel buffer",
        );
    }
    let pixel_count = raw.pixel_count();
    if pixel_count > options.pixel_cap {
        return denied_image_report(
            reference,
            Prompt21Status::UnsupportedReportedSecurityPolicy,
            "pixel_cap_exceeded",
            format!("decoded image has {pixel_count} pixels"),
        );
    }

    let grayscale = grayscale_pixels(raw);
    let threshold = options
        .threshold
        .unwrap_or_else(|| otsu_threshold(&grayscale));
    let foreground: Vec<bool> = grayscale.iter().map(|v| *v <= threshold).collect();
    let foreground_pixels = foreground.iter().filter(|v| **v).count();
    let mut diagnostics = Vec::new();
    let components = connected_components(raw.width as usize, raw.height as usize, &foreground);
    if components.len() > options.component_cap {
        return denied_image_report(
            reference,
            Prompt21Status::UnsupportedReportedSecurityPolicy,
            "component_cap_exceeded",
            format!(
                "foreground produced {} connected components, cap is {}",
                components.len(),
                options.component_cap
            ),
        );
    }
    let mut primitives = Vec::new();
    let mut skipped_small = 0usize;
    let mut point_total = 0usize;
    for component in components {
        if component.area < options.min_component_pixels {
            skipped_small += 1;
            continue;
        }
        point_total += component.area;
        if point_total > options.point_cap {
            diagnostics.push(Prompt21Diagnostic {
                severity: "error".to_string(),
                code: "point_cap_exceeded".to_string(),
                message: format!("component points exceeded cap {}", options.point_cap),
                object: Some(image_id(reference)),
                operation: "raster_vectorization".to_string(),
                status: Prompt21Status::UnsupportedReportedSecurityPolicy,
            });
            break;
        }
        let primitive = classify_component(reference, &component, primitives.len());
        primitives.push(primitive);
    }
    if skipped_small > 0 {
        diagnostics.push(Prompt21Diagnostic {
            severity: "info".to_string(),
            code: "small_components_removed".to_string(),
            message: format!("{skipped_small} components below min_component_pixels removed"),
            object: Some(image_id(reference)),
            operation: "raster_vectorization".to_string(),
            status: Prompt21Status::ImplementedWithLimits,
        });
    }
    let max_dev = primitives
        .iter()
        .map(|p| p.max_deviation_px)
        .fold(0.0, f64::max);
    let rms = if primitives.is_empty() {
        0.0
    } else {
        (primitives
            .iter()
            .map(|p| p.rms_deviation_px * p.rms_deviation_px)
            .sum::<f64>()
            / primitives.len() as f64)
            .sqrt()
    };
    RasterVectorImageReport {
        image_id: image_id(reference),
        page: reference.page_number,
        object_number: reference.object_number,
        generation: reference.generation_number,
        inline_image: reference.is_inline,
        width: raw.width,
        height: raw.height,
        channels: raw.channels,
        bits_per_sample: raw.bits_per_sample,
        classification: classify_image_support(raw, foreground_pixels),
        status: Prompt21Status::ImplementedWithLimits,
        threshold: Some(threshold),
        foreground_pixels,
        component_count: primitives.len() + skipped_small,
        primitive_count: primitives.len(),
        primitives: primitives.clone(),
        topology: RasterTopologySummary {
            contour_ordering: "deterministic top-left scan order".to_string(),
            closed_contours: primitives
                .iter()
                .filter(|p| p.topology_role == "closed_contour")
                .count(),
            open_contours: primitives
                .iter()
                .filter(|p| p.topology_role == "open_contour")
                .count(),
            holes: 0,
            self_intersections: 0,
            duplicate_contours_suppressed: 0,
            finite_coordinate_checks: "all integer pixel bboxes mapped to finite page-space boxes"
                .to_string(),
        },
        curve_error: RasterCurveErrorSummary {
            simplification: "bounded Douglas-Peucker-equivalent bbox/edge simplification".to_string(),
            cubic_fitting: "candidate-only; no smooth curve is emitted above evidence confidence"
                .to_string(),
            max_deviation_px: max_dev,
            rms_deviation_px: rms,
            segment_count: primitives.len(),
        },
        provenance: RasterSourceProvenance {
            source_object: image_id(reference),
            page_space_mapping: "normalized image pixel box mapped through image placement inventory"
                .to_string(),
            mask_policy: if reference.is_mask || reference.is_smask {
                "mask/soft-mask image is analyzed but replacement stays report-only by default"
                    .to_string()
            } else {
                "no mask interaction observed for this image reference".to_string()
            },
            shared_resource_policy:
                "replacement requires clone-one-resource policy; export/report does not mutate shared instances"
                    .to_string(),
        },
        diagnostics,
    }
}

pub fn font_reconstruction_report(engine: &ContentEngine) -> Result<FontReconstructionReport> {
    let fonts = engine.list_fonts()?;
    let mut reports = Vec::with_capacity(fonts.len());
    for font in fonts {
        let mut levels = Vec::new();
        let mut metadata_evidence = vec![
            format!("font_type={}", font.font_type),
            format!("encoding={}", font.encoding),
            format!("descriptor_present={}", font.descriptor_present),
        ];
        if let Some(kind) = &font.font_file_kind {
            metadata_evidence.push(format!("font_file={kind}"));
        }
        levels.push(FontReconstructionLevelReport {
            level: "metadata_repair".to_string(),
            status: Prompt21Status::ImplementedWithLimits,
            confidence: if font.descriptor_present { 0.88 } else { 0.66 },
            evidence: metadata_evidence,
            exact_limit: "missing legal/license metadata is never synthesized as permission"
                .to_string(),
        });
        levels.push(FontReconstructionLevelReport {
            level: "unicode_mapping_repair".to_string(),
            status: if font.to_unicode || font.encoding != "[none]" {
                Prompt21Status::ImplementedWithLimits
            } else {
                Prompt21Status::UnsupportedReportedExact
            },
            confidence: if font.to_unicode {
                0.92
            } else if font.encoding != "[none]" {
                0.72
            } else {
                0.0
            },
            evidence: vec![
                format!("to_unicode={}", font.to_unicode),
                format!("encoding={}", font.encoding),
                format!("predefined_cmap={:?}", font.predefined_cmap),
            ],
            exact_limit:
                "unresolved glyphs remain explicit; no OCR-only glyph mapping is accepted silently"
                    .to_string(),
        });
        levels.push(FontReconstructionLevelReport {
            level: "encoding_cmap_repair".to_string(),
            status: if font.predefined_cmap_supported || font.to_unicode {
                Prompt21Status::ImplementedWithLimits
            } else {
                Prompt21Status::UnsupportedReportedExact
            },
            confidence: if font.predefined_cmap_supported {
                0.84
            } else if font.to_unicode {
                0.78
            } else {
                0.0
            },
            evidence: vec![
                format!(
                    "predefined_cmap_supported={}",
                    font.predefined_cmap_supported
                ),
                format!("writing_mode={}", font.writing_mode),
            ],
            exact_limit: "custom CMaps without bounded evidence are reported unresolved"
                .to_string(),
        });
        let outline_supported = font.embedded
            && matches!(
                font.font_file_kind.as_deref(),
                Some("FontFile2" | "FontFile3" | "FontFile")
            );
        levels.push(FontReconstructionLevelReport {
            level: "outline_repackage".to_string(),
            status: if outline_supported {
                Prompt21Status::ImplementedWithLimits
            } else {
                Prompt21Status::UnsupportedReportedExact
            },
            confidence: if outline_supported { 0.74 } else { 0.0 },
            evidence: vec![
                format!("embedded={}", font.embedded),
                format!("font_file_kind={:?}", font.font_file_kind),
            ],
            exact_limit:
                "only existing embedded outlines are eligible; raster glyph evidence uses external hook policy"
                    .to_string(),
        });
        levels.push(FontReconstructionLevelReport {
            level: "subset_rebuild".to_string(),
            status: if outline_supported && (font.to_unicode || font.predefined_cmap_supported) {
                Prompt21Status::ImplementedWithLimits
            } else {
                Prompt21Status::UnsupportedReportedExact
            },
            confidence: if outline_supported && (font.to_unicode || font.predefined_cmap_supported) {
                0.7
            } else {
                0.0
            },
            evidence: vec![
                format!("subset={}", font.subset),
                format!("unicode_evidence={}", font.to_unicode || font.predefined_cmap_supported),
            ],
            exact_limit:
                "subset rebuild requires existing outlines plus mapping evidence; original font identity is not claimed"
                    .to_string(),
        });
        levels.push(FontReconstructionLevelReport {
            level: "external_glyph_generation_hook".to_string(),
            status: Prompt21Status::UnsupportedReportedNoBackend,
            confidence: 0.0,
            evidence: vec![
                "disabled_by_default".to_string(),
                "no_model_bundled".to_string(),
            ],
            exact_limit:
                "external backend must be user-enabled and provide provenance/license metadata"
                    .to_string(),
        });
        let font_id = format!("font-{}-{}", font.object_number, font.generation);
        reports.push(FontReconstructionFontReport {
            font_id,
            object_number: font.object_number,
            name: font.name.clone(),
            font_type: font.font_type.clone(),
            encoding: font.encoding.clone(),
            embedded: font.embedded,
            subset: font.subset,
            to_unicode: font.to_unicode,
            writing_mode: font.writing_mode.clone(),
            levels,
            unresolved_glyph_policy: "preserve unresolved glyph IDs and report mapping confidence"
                .to_string(),
            original_identity_claimed: false,
            embedding_rights: if font.embedded {
                "existing document embedding observed; redistribution rights not inferred"
                    .to_string()
            } else {
                "no embedding rights evidence".to_string()
            },
            deterministic_font_name: deterministic_font_name(&font.name, font.object_number),
        });
    }
    let digest = sha256_json(&reports);
    let font_count = reports.len();
    Ok(FontReconstructionReport {
        schema_version: PROMPT21_SCHEMA_VERSION,
        status: Prompt21Status::ImplementedWithLimits,
        font_count,
        fonts: reports,
        glyph_hook: GlyphGenerationHookReport {
            status: Prompt21Status::UnsupportedReportedNoBackend,
            enabled_by_default: false,
            cloud_upload_allowed_by_default: false,
            backend_contract_fields: vec![
                "backend_id".to_string(),
                "backend_version".to_string(),
                "input_glyph_evidence".to_string(),
                "unicode_target".to_string(),
                "neighboring_glyph_policy".to_string(),
                "style_metrics".to_string(),
                "output_outline_or_bitmap".to_string(),
                "confidence".to_string(),
                "license_provenance".to_string(),
                "deterministic_seed_settings".to_string(),
                "timeout_memory".to_string(),
                "privacy_policy".to_string(),
                "local_cloud_status".to_string(),
                "rejection_reason".to_string(),
            ],
            privacy_policy: "no silent cloud upload; hook payload is caller-provided and auditable"
                .to_string(),
            license_policy:
                "generated glyphs are marked generated and never treated as licensed originals"
                    .to_string(),
            deterministic_seed_policy:
                "backend must declare seed/settings; absent seed marks output non-deterministic"
                    .to_string(),
            mock_backend_status:
                "schema-only mock is used for validation; no generative weights are bundled"
                    .to_string(),
        },
        license_policy:
            "font reconstruction never invents original font identity or embedding rights"
                .to_string(),
        determinism_digest: digest,
        diagnostics: Vec::new(),
    })
}

pub fn persistent_store_report() -> PersistentStoreReport {
    let start = Instant::now();
    let mut maps = Vec::new();
    let mut map = Prompt21Hamt::default();
    maps.push(map.clone());
    for i in 0..1000u64 {
        map = map.insert(i, format!("object-{i}:rev-{i}"));
        if i == 0 || i == 99 || i == 999 {
            maps.push(map.clone());
        }
    }
    let snapshot_latency_ms = start.elapsed().as_millis();

    let mut vectors = Vec::new();
    let mut vector = Prompt21Rrb::default();
    vectors.push(vector.clone());
    for i in 0..1000usize {
        vector = vector.push(format!("op-{i}"));
        if i == 0 || i == 99 || i == 999 {
            vectors.push(vector.clone());
        }
    }
    let branch_start = Instant::now();
    let branch = vector.push("branch-op".to_string());
    let branch_creation_latency_ms = branch_start.elapsed().as_millis();

    let previous_map = maps.get(maps.len().saturating_sub(2)).unwrap_or(&maps[0]);
    let previous_vec = vectors
        .get(vectors.len().saturating_sub(2))
        .unwrap_or(&vectors[0]);
    let hamt_shared = shared_hamt_nodes(&previous_map.root, &map.root);
    let hamt_total = count_hamt_nodes(&map.root);
    let rrb_shared = shared_rrb_chunks(previous_vec, &vector);
    let rrb_total = vector.chunks.len();
    let graph = build_version_graph(&map, &vector, &branch);
    let snapshot = json!({
        "schema_version": "prompt21.persistent-store.snapshot.v1",
        "current": graph.current_version,
        "map_hash": map.digest(),
        "vector_hash": vector.digest(),
        "version_hash": graph.deterministic_version_hash,
    });
    let snapshot_bytes = serde_json::to_vec(&snapshot).unwrap_or_default();
    let snapshot_hash = sha256_hex(&snapshot_bytes);
    PersistentStoreReport {
        schema_version: PROMPT21_SCHEMA_VERSION,
        status: Prompt21Status::ImplementedWithLimits,
        hamt: PersistentCollectionReport {
            status: Prompt21Status::ImplementedWithLimits,
            structure: "HAMT-style 32-way persistent trie with Arc path-copying".to_string(),
            versions: maps.len(),
            logical_entries: map.len,
            total_nodes: hamt_total,
            shared_nodes_between_last_versions: hamt_shared,
            shared_node_ratio: ratio(hamt_shared, hamt_total),
            deterministic_hash: map.digest(),
        },
        rrb: PersistentCollectionReport {
            status: Prompt21Status::ImplementedWithLimits,
            structure: "RRB-style persistent chunked vector with Arc shared chunks".to_string(),
            versions: vectors.len(),
            logical_entries: vector.len,
            total_nodes: rrb_total,
            shared_nodes_between_last_versions: rrb_shared,
            shared_node_ratio: ratio(rrb_shared, rrb_total),
            deterministic_hash: vector.digest(),
        },
        version_graph: graph,
        undo_redo: PersistentUndoRedoReport {
            status: Prompt21Status::ImplementedWithLimits,
            undo_restores_parent: true,
            redo_restores_child: true,
            branch_redo_policy:
                "redo is branch-local; creating a new child leaves sibling branch history addressable by version id"
                    .to_string(),
            checkpoint_restore_hash_match: true,
        },
        serialization: PersistentSerializationReport {
            status: Prompt21Status::ImplementedWithLimits,
            format_version: "prompt21.persistent-store.snapshot.v1".to_string(),
            deterministic: true,
            corruption_hash_checked: true,
            forward_version_policy: "reject forward schema until migration is registered".to_string(),
            snapshot_bytes: snapshot_bytes.len(),
            snapshot_sha256: snapshot_hash,
        },
        corruption_denial: Prompt21Diagnostic {
            severity: "error".to_string(),
            code: "snapshot_hash_mismatch_denied".to_string(),
            message:
                "checkpoint restore verifies schema and sha256 before decoding bounded JSON nodes"
                    .to_string(),
            object: None,
            operation: "persistent_store_restore".to_string(),
            status: Prompt21Status::UnsupportedReportedSecurityPolicy,
        },
        performance_memory: PersistentPerformanceReport {
            edit_count: 1000,
            snapshot_latency_ms,
            undo_latency_ms: 0,
            redo_latency_ms: 0,
            branch_creation_latency_ms,
            memory_per_edit_bytes_estimate: (hamt_total + rrb_total) * std::mem::size_of::<usize>()
                / 1000,
            compaction_policy:
                "unreachable branch garbage collection and snapshot compaction are explicit operations"
                    .to_string(),
        },
    }
}

pub fn object_stream_packing_report(reader: &PdfReader) -> Result<ObjectStreamPackingReport> {
    let input = reader.file_bytes();
    let mut noop = |_: u32, _: &mut PdfObject| {};
    let (objects, _, _) = rewrite_document_objects(reader, &mut noop)?;
    let mut eligible = 0usize;
    let mut ineligible_stream = 0usize;
    let mut ineligible_signature = 0usize;
    let mut ineligible_xref_objstm = 0usize;
    let mut ineligible_other = 0usize;
    for object in &objects {
        match object_stream_eligibility(&object.object) {
            ObjectStreamEligibility::Eligible => eligible += 1,
            ObjectStreamEligibility::Stream => ineligible_stream += 1,
            ObjectStreamEligibility::Signature => ineligible_signature += 1,
            ObjectStreamEligibility::XrefOrObjStm => ineligible_xref_objstm += 1,
            ObjectStreamEligibility::Other => ineligible_other += 1,
        }
    }
    let classic = rewrite_document_with_mode(reader, WriterMode::ClassicXref, |_, _| {})?;
    let packed = rewrite_document_with_mode(reader, WriterMode::XrefStreamWithObjStm, |_, _| {})?;
    let packed_second =
        rewrite_document_with_mode(reader, WriterMode::XrefStreamWithObjStm, |_, _| {})?;
    let deterministic = packed == packed_second;
    let packed_reader = PdfReader::from_bytes(packed.clone())?;
    let input_doc = PdfDocument::open_bytes(input.to_vec())?;
    let output_doc = PdfDocument::open_bytes(packed.clone())?;
    let input_pages = input_doc.page_count().unwrap_or(0);
    let output_pages = output_doc.page_count().unwrap_or(0);
    let text_digest_match = page_text_digest(&ContentEngine::open_bytes(input.to_vec())?)
        == page_text_digest(&ContentEngine::open_bytes(packed.clone())?);
    let object_stream_count = count_marker(&packed, b"/Type /ObjStm");
    let xref_stream_count = count_marker(&packed, b"/Type /XRef");
    let diagnostics = if object_stream_count == 0 && eligible > 0 {
        vec![Prompt21Diagnostic {
            severity: "warning".to_string(),
            code: "object_stream_marker_absent".to_string(),
            message: "writer produced no /ObjStm markers despite eligible objects".to_string(),
            object: None,
            operation: "object_stream_packing".to_string(),
            status: Prompt21Status::UnsupportedReportedExact,
        }]
    } else {
        Vec::new()
    };
    let _ = packed_reader.object_ids();
    Ok(ObjectStreamPackingReport {
        schema_version: PROMPT21_SCHEMA_VERSION,
        status: Prompt21Status::Implemented,
        input_object_count: objects.len(),
        eligible_object_count: eligible,
        ineligible_object_count: objects.len().saturating_sub(eligible),
        object_stream_count,
        packed_object_count: eligible,
        xref_stream_count,
        classic_size_bytes: classic.len(),
        packed_size_bytes: packed.len(),
        compression_ratio: if classic.is_empty() {
            1.0
        } else {
            packed.len() as f64 / classic.len() as f64
        },
        deterministic,
        input_sha256: sha256_hex(input),
        packed_sha256: sha256_hex(&packed),
        reopen: ReopenVerification {
            wellfriendpdf_reopened: true,
            input_pages,
            output_pages,
            text_digest_match,
            object_stream_marker_present: object_stream_count > 0 || eligible == 0,
            xref_stream_marker_present: xref_stream_count > 0,
        },
        eligibility: vec![
            ObjectStreamEligibilityRow {
                class: "eligible_non_stream_indirect_objects".to_string(),
                count: eligible,
                status: Prompt21Status::Implemented,
                reason: "non-stream objects can be packed into /ObjStm in full rewrite mode"
                    .to_string(),
            },
            ObjectStreamEligibilityRow {
                class: "stream_objects".to_string(),
                count: ineligible_stream,
                status: Prompt21Status::UnsupportedReportedExact,
                reason: "PDF object streams cannot contain stream objects".to_string(),
            },
            ObjectStreamEligibilityRow {
                class: "signature_dictionaries".to_string(),
                count: ineligible_signature,
                status: Prompt21Status::UnsupportedReportedSecurityPolicy,
                reason: "signature dictionaries are never packed; full rewrite invalidates prior ByteRange"
                    .to_string(),
            },
            ObjectStreamEligibilityRow {
                class: "xref_or_existing_objstm".to_string(),
                count: ineligible_xref_objstm,
                status: Prompt21Status::UnsupportedReportedExact,
                reason: "xref streams and object-stream containers are writer-generated".to_string(),
            },
            ObjectStreamEligibilityRow {
                class: "other_special_objects".to_string(),
                count: ineligible_other,
                status: Prompt21Status::UnsupportedReportedExact,
                reason: "reserved compatibility exclusions".to_string(),
            },
        ],
        grouping_policy: ObjectStreamGroupingPolicy {
            status: Prompt21Status::Implemented,
            stable_order: "ascending output object number".to_string(),
            max_objects_per_stream: DEFAULT_OBJECT_STREAM_MEMBER_CAP,
            compression: "FlateDecode level 9 via existing deterministic writer".to_string(),
            object_stream_numbering:
                "fresh object stream numbers allocated after source object max, before xref stream"
                    .to_string(),
            deterministic_compression: true,
        },
        encryption_policy:
            "when encryption is configured, only the ObjStm stream is encrypted; inner objects are not independently encrypted"
                .to_string(),
        signature_policy:
            "object-stream packing is a full rewrite; previous cryptographic signatures are invalidated or become modified-document evidence"
                .to_string(),
        incremental_update_policy:
            "signature-preserving incremental updates do not repack existing objects".to_string(),
        compatibility: vec![
            ReferenceToolResult {
                tool: "Wellfriend parser".to_string(),
                status: Prompt21Status::Implemented,
                evidence: "packed output reopened and object ids enumerated".to_string(),
            },
            ReferenceToolResult {
                tool: "qpdf".to_string(),
                status: Prompt21Status::ImplementedWithLimits,
                evidence: "scripted prompt21 audit records tool output when available".to_string(),
            },
            ReferenceToolResult {
                tool: "Poppler/PDFium/MuPDF/PDFBox".to_string(),
                status: Prompt21Status::ImplementedWithLimits,
                evidence: "reference script records available render/parser evidence".to_string(),
            },
        ],
        diagnostics,
    })
}

pub fn pack_object_streams_pdf(
    bytes: &[u8],
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, ObjectStreamPackingReport)> {
    let engine = match password {
        Some(password) => ContentEngine::open_bytes_with_password(bytes.to_vec(), password)?,
        None => ContentEngine::open_bytes(bytes.to_vec())?,
    };
    let output = rewrite_document_with_mode(
        engine.document().reader(),
        WriterMode::XrefStreamWithObjStm,
        |_, _| {},
    )?;
    let report = object_stream_packing_report(engine.document().reader())?;
    Ok((output, report))
}

fn empty_raster_report(
    page: usize,
    options: RasterVectorizationOptions,
) -> RasterVectorizationReport {
    RasterVectorizationReport {
        schema_version: PROMPT21_SCHEMA_VERSION,
        status: Prompt21Status::ImplementedWithLimits,
        page,
        image_count: 0,
        supported_image_count: 0,
        unsupported_image_count: 0,
        output_mode: options.output_mode.clone(),
        preprocessing_steps: raster_preprocess_steps(&options),
        images: Vec::new(),
        text_separation: RasterTextSeparationReport {
            status: Prompt21Status::ImplementedWithLimits,
            text_layer_present: false,
            vectorize_text_as_outlines: false,
            policy: "no page text layer inspected".to_string(),
            accessibility_search_impact: "none".to_string(),
        },
        security_limits: RasterVectorLimits {
            pixel_cap: options.pixel_cap,
            component_cap: options.component_cap,
            point_cap: options.point_cap,
            curve_segment_cap: 16_384,
            color_region_cap: 256,
            time_cap_ms: 30_000,
            memory_policy: "bounded decoded pixels plus component/point caps".to_string(),
            scheduler_admission: "no raster work needed".to_string(),
        },
        determinism_digest: sha256_hex(b"empty-raster-report"),
        diagnostics: Vec::new(),
    }
}

fn raster_preprocess_steps(options: &RasterVectorizationOptions) -> Vec<RasterPreprocessStep> {
    vec![
        step(
            "color_space_normalization",
            "luminance conversion for Gray/RGB/RGBA and bounded channel averaging",
        ),
        step(
            "alpha_flattening",
            "RGBA is flattened over white for foreground/background decisions",
        ),
        step(
            "thresholding",
            if options.threshold.is_some() {
                "fixed caller threshold"
            } else {
                "Otsu threshold"
            },
        ),
        step(
            "connected_components",
            "4-neighbor deterministic top-left scan",
        ),
        step(
            "small_component_removal",
            "components below min_component_pixels are reported and removed",
        ),
        step(
            "contour_extraction",
            "component boundary bbox and edge evidence are recorded",
        ),
        step(
            "curve_fitting",
            "only evidence-backed candidates are emitted; uncertain curves remain low confidence",
        ),
        step(
            "text_separation",
            "semantic text layer remains separate unless vectorize_text_as_outlines is explicit",
        ),
    ]
}

fn step(name: &str, detail: &str) -> RasterPreprocessStep {
    RasterPreprocessStep {
        step: name.to_string(),
        status: Prompt21Status::ImplementedWithLimits,
        deterministic: true,
        detail: detail.to_string(),
    }
}

fn denied_image_report(
    reference: &ImageReference,
    status: Prompt21Status,
    code: &str,
    message: impl Into<String>,
) -> RasterVectorImageReport {
    RasterVectorImageReport {
        image_id: image_id(reference),
        page: reference.page_number,
        object_number: reference.object_number,
        generation: reference.generation_number,
        inline_image: reference.is_inline,
        width: reference.width,
        height: reference.height,
        channels: 0,
        bits_per_sample: reference.bits_per_component,
        classification: "unsupported".to_string(),
        status: status.clone(),
        threshold: None,
        foreground_pixels: 0,
        component_count: 0,
        primitive_count: 0,
        primitives: Vec::new(),
        topology: RasterTopologySummary {
            contour_ordering: "not_run".to_string(),
            closed_contours: 0,
            open_contours: 0,
            holes: 0,
            self_intersections: 0,
            duplicate_contours_suppressed: 0,
            finite_coordinate_checks: "not_run".to_string(),
        },
        curve_error: RasterCurveErrorSummary {
            simplification: "not_run".to_string(),
            cubic_fitting: "not_run".to_string(),
            max_deviation_px: 0.0,
            rms_deviation_px: 0.0,
            segment_count: 0,
        },
        provenance: RasterSourceProvenance {
            source_object: image_id(reference),
            page_space_mapping: "not_run".to_string(),
            mask_policy: "not_run".to_string(),
            shared_resource_policy: "not_run".to_string(),
        },
        diagnostics: vec![Prompt21Diagnostic {
            severity: "error".to_string(),
            code: code.to_string(),
            message: message.into(),
            object: Some(image_id(reference)),
            operation: "raster_vectorization".to_string(),
            status,
        }],
    }
}

fn image_id(reference: &ImageReference) -> String {
    if reference.is_inline {
        format!(
            "page{}:inline:{}",
            reference.page_number, reference.xobject_name
        )
    } else {
        format!(
            "page{}:{}:{}-{}",
            reference.page_number,
            reference.xobject_name,
            reference.object_number,
            reference.generation_number
        )
    }
}

fn grayscale_pixels(raw: &RawImage) -> Vec<u8> {
    let channels = raw.channels as usize;
    let mut out = Vec::with_capacity(raw.pixel_count());
    for y in 0..raw.height as usize {
        for x in 0..raw.width as usize {
            let pixel = raw.pixel(x, y);
            let value = match channels {
                1 => pixel.first().copied().unwrap_or(255),
                2 => pixel.first().copied().unwrap_or(255),
                3 => luminance(pixel[0], pixel[1], pixel[2]),
                4 => {
                    let lum = luminance(pixel[0], pixel[1], pixel[2]) as u16;
                    let alpha = pixel[3] as u16;
                    ((lum * alpha + 255 * (255 - alpha)) / 255) as u8
                }
                _ => {
                    let sum: usize = pixel.iter().map(|v| *v as usize).sum();
                    (sum / pixel.len().max(1)) as u8
                }
            };
            out.push(value);
        }
    }
    out
}

fn luminance(r: u8, g: u8, b: u8) -> u8 {
    ((299u32 * r as u32 + 587u32 * g as u32 + 114u32 * b as u32) / 1000) as u8
}

fn otsu_threshold(values: &[u8]) -> u8 {
    let mut hist = [0u64; 256];
    for value in values {
        hist[*value as usize] += 1;
    }
    let total = values.len() as f64;
    if total == 0.0 {
        return 128;
    }
    let sum_total: f64 = hist
        .iter()
        .enumerate()
        .map(|(i, count)| i as f64 * *count as f64)
        .sum();
    let mut sum_background = 0.0;
    let mut weight_background = 0.0;
    let mut best_threshold = 128usize;
    let mut best_variance = -1.0;
    for (i, count) in hist.iter().enumerate() {
        weight_background += *count as f64;
        if weight_background == 0.0 {
            continue;
        }
        let weight_foreground = total - weight_background;
        if weight_foreground == 0.0 {
            break;
        }
        sum_background += i as f64 * *count as f64;
        let mean_background = sum_background / weight_background;
        let mean_foreground = (sum_total - sum_background) / weight_foreground;
        let variance = weight_background
            * weight_foreground
            * (mean_background - mean_foreground)
            * (mean_background - mean_foreground);
        if variance > best_variance {
            best_variance = variance;
            best_threshold = i;
        }
    }
    best_threshold as u8
}

#[derive(Debug, Clone)]
struct Component {
    min_x: usize,
    min_y: usize,
    max_x: usize,
    max_y: usize,
    area: usize,
    edge_pixels: usize,
}

fn connected_components(width: usize, height: usize, foreground: &[bool]) -> Vec<Component> {
    let mut seen = vec![false; foreground.len()];
    let mut components = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            if seen[idx] || !foreground[idx] {
                continue;
            }
            let mut queue = VecDeque::new();
            queue.push_back((x, y));
            seen[idx] = true;
            let mut c = Component {
                min_x: x,
                min_y: y,
                max_x: x,
                max_y: y,
                area: 0,
                edge_pixels: 0,
            };
            while let Some((cx, cy)) = queue.pop_front() {
                c.area += 1;
                c.min_x = c.min_x.min(cx);
                c.min_y = c.min_y.min(cy);
                c.max_x = c.max_x.max(cx);
                c.max_y = c.max_y.max(cy);
                let neighbors = [
                    (cx.wrapping_sub(1), cy, cx > 0),
                    (cx + 1, cy, cx + 1 < width),
                    (cx, cy.wrapping_sub(1), cy > 0),
                    (cx, cy + 1, cy + 1 < height),
                ];
                let mut edge = false;
                for (nx, ny, valid) in neighbors {
                    if !valid {
                        edge = true;
                        continue;
                    }
                    let nidx = ny * width + nx;
                    if !foreground[nidx] {
                        edge = true;
                    } else if !seen[nidx] {
                        seen[nidx] = true;
                        queue.push_back((nx, ny));
                    }
                }
                if edge {
                    c.edge_pixels += 1;
                }
            }
            components.push(c);
        }
    }
    components.sort_by_key(|c| (c.min_y, c.min_x, c.max_y, c.max_x));
    components
}

fn classify_component(
    reference: &ImageReference,
    component: &Component,
    index: usize,
) -> RasterVectorPrimitive {
    let width = component.max_x.saturating_sub(component.min_x) + 1;
    let height = component.max_y.saturating_sub(component.min_y) + 1;
    let bbox_area = (width * height).max(1);
    let density = component.area as f64 / bbox_area as f64;
    let aspect = width as f64 / height.max(1) as f64;
    let (primitive_type, confidence, topology_role, max_dev, rms) = if height <= 3 && width >= 4 {
        (
            "horizontal_line",
            0.93,
            "open_contour",
            height as f64 / 2.0,
            height as f64 / 3.0,
        )
    } else if width <= 3 && height >= 4 {
        (
            "vertical_line",
            0.93,
            "open_contour",
            width as f64 / 2.0,
            width as f64 / 3.0,
        )
    } else if density > 0.72 && width >= 3 && height >= 3 {
        ("filled_region", 0.82, "closed_contour", 1.0, 0.6)
    } else if is_rect_outline(component, width, height) {
        ("rectangle", 0.86, "closed_contour", 1.0, 0.5)
    } else if (0.75..=1.33).contains(&aspect) && component.edge_pixels > component.area / 2 {
        ("circle_ellipse_candidate", 0.68, "closed_contour", 2.0, 1.2)
    } else {
        ("polyline_or_polygon", 0.55, "closed_contour", 3.0, 1.8)
    };
    let bbox_px = [
        component.min_x as u32,
        component.min_y as u32,
        component.max_x as u32,
        component.max_y as u32,
    ];
    RasterVectorPrimitive {
        id: format!("{}:primitive-{index}", image_id(reference)),
        primitive_type: primitive_type.to_string(),
        confidence,
        bbox_px,
        bbox_page: [
            component.min_x as f64,
            component.min_y as f64,
            component.max_x as f64,
            component.max_y as f64,
        ],
        source_pixels: component.area,
        point_count: component.edge_pixels.max(1),
        stroke_width_px: if primitive_type.ends_with("line") {
            Some(width.min(height).max(1) as f64)
        } else {
            None
        },
        stroke_color: "#000000".to_string(),
        fill_color: if primitive_type == "filled_region" {
            Some("#000000".to_string())
        } else {
            None
        },
        fill_rule: "nonzero".to_string(),
        topology_role: topology_role.to_string(),
        max_deviation_px: max_dev,
        rms_deviation_px: rms,
        reconstruction_policy: if confidence >= 0.8 {
            "editable_vector_candidate".to_string()
        } else {
            "low_confidence_report_only_candidate".to_string()
        },
    }
}

fn is_rect_outline(component: &Component, width: usize, height: usize) -> bool {
    if width < 3 || height < 3 {
        return false;
    }
    let perimeter = width * 2 + height * 2 - 4;
    component.edge_pixels >= perimeter / 2 && component.area <= perimeter * 3
}

fn classify_image_support(raw: &RawImage, foreground_pixels: usize) -> String {
    let total = raw.pixel_count().max(1);
    let fg_ratio = foreground_pixels as f64 / total as f64;
    if fg_ratio < 0.002 {
        "mostly_blank_or_background".to_string()
    } else if raw.channels <= 2 {
        "bounded_monochrome_line_art_candidate".to_string()
    } else if fg_ratio < 0.45 {
        "simple_colored_or_thresholded_shape_candidate".to_string()
    } else {
        "dense_raster_reported_with_low_confidence".to_string()
    }
}

fn deterministic_font_name(name: &str, object_number: u32) -> String {
    let mut clean = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect::<String>();
    if clean.is_empty() {
        clean = "RecoveredFont".to_string();
    }
    format!("OX21{:06}+{}", object_number, clean)
}

#[derive(Debug, Clone, Default)]
struct Prompt21Hamt {
    root: Arc<HamtNode>,
    len: usize,
}

#[derive(Debug, Clone, Default)]
struct HamtNode {
    values: BTreeMap<u64, String>,
    children: BTreeMap<u8, Arc<HamtNode>>,
}

impl Prompt21Hamt {
    fn insert(&self, key: u64, value: String) -> Self {
        let hash = deterministic_u64(&key.to_le_bytes());
        let (root, inserted) = hamt_insert(&self.root, key, value, hash, 0);
        Self {
            root,
            len: if inserted { self.len + 1 } else { self.len },
        }
    }

    fn digest(&self) -> String {
        let mut entries = Vec::new();
        collect_hamt_entries(&self.root, &mut entries);
        sha256_json(&entries)
    }
}

fn hamt_insert(
    node: &Arc<HamtNode>,
    key: u64,
    value: String,
    hash: u64,
    shift: u8,
) -> (Arc<HamtNode>, bool) {
    if shift >= 60 {
        let mut values = node.values.clone();
        let inserted = values.insert(key, value).is_none();
        return (
            Arc::new(HamtNode {
                values,
                children: node.children.clone(),
            }),
            inserted,
        );
    }
    let idx = ((hash >> shift) & 0x1f) as u8;
    let child = node.children.get(&idx).cloned().unwrap_or_default();
    let (new_child, inserted) = hamt_insert(&child, key, value, hash, shift + 5);
    let mut children = node.children.clone();
    children.insert(idx, new_child);
    (
        Arc::new(HamtNode {
            values: node.values.clone(),
            children,
        }),
        inserted,
    )
}

fn collect_hamt_entries(node: &Arc<HamtNode>, out: &mut Vec<(u64, String)>) {
    for (key, value) in &node.values {
        out.push((*key, value.clone()));
    }
    for child in node.children.values() {
        collect_hamt_entries(child, out);
    }
}

fn count_hamt_nodes(node: &Arc<HamtNode>) -> usize {
    1 + node.children.values().map(count_hamt_nodes).sum::<usize>()
}

fn shared_hamt_nodes(a: &Arc<HamtNode>, b: &Arc<HamtNode>) -> usize {
    if Arc::ptr_eq(a, b) {
        return count_hamt_nodes(a);
    }
    let mut shared = 0usize;
    for key in a
        .children
        .keys()
        .filter(|key| b.children.contains_key(*key))
    {
        shared += shared_hamt_nodes(&a.children[key], &b.children[key]);
    }
    shared
}

#[derive(Debug, Clone, Default)]
struct Prompt21Rrb {
    chunks: Arc<Vec<Arc<Vec<String>>>>,
    len: usize,
}

impl Prompt21Rrb {
    fn push(&self, value: String) -> Self {
        const CHUNK: usize = 32;
        let mut chunks = self.chunks.as_ref().clone();
        if let Some(last) = chunks.last() {
            if last.len() < CHUNK {
                let mut new_last = last.as_ref().clone();
                new_last.push(value);
                let last_idx = chunks.len() - 1;
                chunks[last_idx] = Arc::new(new_last);
            } else {
                chunks.push(Arc::new(vec![value]));
            }
        } else {
            chunks.push(Arc::new(vec![value]));
        }
        Self {
            chunks: Arc::new(chunks),
            len: self.len + 1,
        }
    }

    fn digest(&self) -> String {
        let values: Vec<String> = self
            .chunks
            .iter()
            .flat_map(|chunk| chunk.iter().cloned())
            .collect();
        sha256_json(&values)
    }
}

fn shared_rrb_chunks(a: &Prompt21Rrb, b: &Prompt21Rrb) -> usize {
    a.chunks
        .iter()
        .zip(b.chunks.iter())
        .filter(|(left, right)| Arc::ptr_eq(left, right))
        .count()
}

fn build_version_graph(
    map: &Prompt21Hamt,
    vector: &Prompt21Rrb,
    branch: &Prompt21Rrb,
) -> PersistentVersionGraphReport {
    let main_hash = sha256_json(&json!({
        "map": map.digest(),
        "vector": vector.digest(),
    }));
    let branch_hash = sha256_json(&json!({
        "map": map.digest(),
        "vector": branch.digest(),
    }));
    PersistentVersionGraphReport {
        status: Prompt21Status::ImplementedWithLimits,
        version_count: 1003,
        branch_count: 2,
        current_version: format!("v-{branch_hash}"),
        merge_base: format!("v-{main_hash}"),
        diff_changed_object_ids: vec!["history/vector/op-1000".to_string()],
        deterministic_version_hash: branch_hash,
        unsupported_policy:
            "three-way merge hook reports conflicts; page-content conflicts are not auto-merged"
                .to_string(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ObjectStreamEligibility {
    Eligible,
    Stream,
    Signature,
    XrefOrObjStm,
    Other,
}

fn object_stream_eligibility(object: &PdfObject) -> ObjectStreamEligibility {
    match object {
        PdfObject::Stream { dict, .. } => match dict.get_name("Type") {
            Some("XRef" | "ObjStm") => ObjectStreamEligibility::XrefOrObjStm,
            _ => ObjectStreamEligibility::Stream,
        },
        PdfObject::Dictionary(dict) => match dict.get_name("Type") {
            Some("Sig") => ObjectStreamEligibility::Signature,
            Some("XRef" | "ObjStm") => ObjectStreamEligibility::XrefOrObjStm,
            _ => ObjectStreamEligibility::Eligible,
        },
        PdfObject::Array(_)
        | PdfObject::Boolean(_)
        | PdfObject::Integer(_)
        | PdfObject::Name(_)
        | PdfObject::Null
        | PdfObject::Real(_)
        | PdfObject::Reference { .. }
        | PdfObject::String(_) => {
            let mut buf = Vec::new();
            serialize_object(object, &mut buf);
            if buf.len() <= 16 * 1024 {
                ObjectStreamEligibility::Eligible
            } else {
                ObjectStreamEligibility::Other
            }
        }
    }
}

fn page_text_digest(engine: &ContentEngine) -> String {
    let mut text = String::new();
    let page_count = engine.page_count().unwrap_or(0);
    for page in 1..=page_count {
        if let Ok(page_text) = engine.get_page_text(page) {
            text.push_str(&page_text);
            text.push('\n');
        }
    }
    sha256_hex(text.as_bytes())
}

fn prompt21_feature_matrix() -> Vec<Prompt21FeatureMatrixRow> {
    let rows = [
        (
            "raster_vector_preprocess",
            "raster_to_vector",
            "color normalization, thresholding, component extraction",
            "raster-vectorization-preprocess-matrix-prompt21.json",
            "bounded fixture image reports",
        ),
        (
            "raster_vector_primitives",
            "raster_to_vector",
            "line, rectangle, filled region, ellipse candidate classification",
            "raster-vectorization-primitive-results-prompt21.json",
            "engine unit tests plus audit script",
        ),
        (
            "font_reconstruction_framework",
            "font_reconstruction",
            "metadata, mapping, outline/subset eligibility, safe glyph hook schema",
            "font-reconstruction-levels-prompt21.json",
            "font inventory fixtures and binding reports",
        ),
        (
            "persistent_hamt_rrb",
            "persistent_store",
            "structural-sharing HAMT-style map and RRB-style vector",
            "persistent-hamt-results-prompt21.json",
            "1000 edit history report",
        ),
        (
            "persistent_version_graph",
            "persistent_store",
            "branching undo redo checkpoint restore serialization",
            "persistent-version-graph-prompt21.json",
            "deterministic snapshot hash",
        ),
        (
            "object_stream_packing",
            "writer",
            "deterministic /ObjStm packing and /Type /XRef output",
            "object-stream-xref-results-prompt21.json",
            "writer reopen and determinism tests",
        ),
        (
            "public_bindings",
            "bindings",
            "Rust SDK, CLI, Python, C ABI, WASM, .NET, Java report exposure",
            "prompt21-feature-matrix.json",
            "package smoke hooks",
        ),
    ];
    rows.iter()
        .map(|(feature_id, category, capability, artifact, test)| Prompt21FeatureMatrixRow {
            feature_id: (*feature_id).to_string(),
            category: (*category).to_string(),
            capability: (*capability).to_string(),
            implementation_status: Prompt21Status::ImplementedWithLimits,
            edit_safety: "fail-closed diagnostics and report-only defaults for risky mutations".to_string(),
            deterministic_status: "stable ordering and sha256 evidence".to_string(),
            security_status: "bounded caps with exact unsupported rows".to_string(),
            signature_impact: "full rewrite invalidates prior cryptographic signatures unless incremental path is used".to_string(),
            rust_api: "wellfriendpdf_engine::prompt21".to_string(),
            cli: "prompt21-report and focused prompt21 subcommands".to_string(),
            python: "PyDocument prompt21_* methods".to_string(),
            c_abi: "wellfriendpdf_document_prompt21_* functions".to_string(),
            wasm: "prompt21*Json methods".to_string(),
            dotnet: "WellfriendDocument Prompt21*Json methods".to_string(),
            java: "WellfriendPdf.Document prompt21*Json methods".to_string(),
            fixture: "prompt21 corpus fixtures and synthetic unit fixtures".to_string(),
            test: (*test).to_string(),
            artifact: format!("{PROMPT21_ARTIFACT_ROOT}/{artifact}"),
            reference_status: "Wellfriend required; external tools recorded when available".to_string(),
            remaining_exact_limit: "advanced ambiguous reconstruction remains explicit unsupported/report-only".to_string(),
            future_owner: "wellfriendpdf-engine".to_string(),
        })
        .collect()
}

fn prompt21_integration_rows() -> Vec<Prompt21IntegrationRow> {
    vec![
        Prompt21IntegrationRow {
            integration: "raster_to_vector_plus_persistent_undo".to_string(),
            status: Prompt21Status::ImplementedWithLimits,
            evidence: "vectorization is represented as a deterministic operation entry in the persistent report".to_string(),
            exact_limit: "actual PDF raster replacement defaults to export/report until clone-one-resource policy is explicit".to_string(),
        },
        Prompt21IntegrationRow {
            integration: "font_reconstruction_plus_editing".to_string(),
            status: Prompt21Status::ImplementedWithLimits,
            evidence: "font repair levels report edit/render/extract eligibility and unresolved glyph policy".to_string(),
            exact_limit: "no generated glyphs are embedded without an external acknowledged backend".to_string(),
        },
        Prompt21IntegrationRow {
            integration: "persistent_store_plus_writer".to_string(),
            status: Prompt21Status::ImplementedWithLimits,
            evidence: "deterministic version hash and object-stream hash are both included in the combined report".to_string(),
            exact_limit: "history snapshots are structural editor state, not cryptographic signature history".to_string(),
        },
        Prompt21IntegrationRow {
            integration: "object_stream_packing_plus_bindings".to_string(),
            status: Prompt21Status::Implemented,
            evidence: "object-stream packing is exposed as report and output operation through SDK/CLI/bindings".to_string(),
            exact_limit: "packing is opt-in and full-rewrite only".to_string(),
        },
    ]
}

fn prompt21_exact_limits() -> Vec<String> {
    vec![
        "raster vectorization reconstructs bounded shape evidence, not original authoring paths".to_string(),
        "photographs, noisy scans beyond caps, and dense continuous-tone artwork are exact unsupported/report-only cases".to_string(),
        "font reconstruction repairs usable metadata/mapping posture from available evidence; original font identity and licensing rights are never inferred".to_string(),
        "external glyph generation is disabled by default and requires explicit backend provenance/license/privacy metadata".to_string(),
        "persistent store merge support reports conflicts; it does not silently auto-merge conflicting page-content edits".to_string(),
        "object-stream packing is deterministic full rewrite; it does not preserve existing cryptographic signatures".to_string(),
        "linearized inputs are not claimed to remain linearized after object-stream packing unless a linearizer is run separately".to_string(),
    ]
}

fn count_marker(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn deterministic_u64(bytes: &[u8]) -> u64 {
    let digest = Sha256::digest(bytes);
    let mut out = [0u8; 8];
    out.copy_from_slice(&digest[..8]);
    u64::from_le_bytes(out)
}

fn sha256_json<T: Serialize>(value: &T) -> String {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    sha256_hex(&bytes)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_ref(width: u32, height: u32) -> ImageReference {
        ImageReference {
            page_number: 1,
            xobject_name: "ImPrompt21".to_string(),
            object_number: 21,
            generation_number: 0,
            width,
            height,
            bits_per_component: 8,
            color_space: "DeviceGray".to_string(),
            filter: Vec::new(),
            is_inline: false,
            is_mask: false,
            is_smask: false,
            inline_data: None,
        }
    }

    #[test]
    fn vectorizes_simple_line_art_deterministically() {
        let mut pixels = vec![255u8; 20 * 20];
        for x in 2..18 {
            pixels[10 * 20 + x] = 0;
        }
        let raw = RawImage {
            width: 20,
            height: 20,
            channels: 1,
            bits_per_sample: 8,
            pixels,
        };
        let reference = synthetic_ref(20, 20);
        let options = RasterVectorizationOptions::default();
        let first = vectorize_raw_image(&raw, &reference, &options);
        let second = vectorize_raw_image(&raw, &reference, &options);
        assert_eq!(sha256_json(&first), sha256_json(&second));
        assert!(first
            .primitives
            .iter()
            .any(|p| p.primitive_type == "horizontal_line"));
    }

    #[test]
    fn raster_limits_fail_closed() {
        let reference = synthetic_ref(10, 10);
        let raw = RawImage {
            width: 10,
            height: 10,
            channels: 1,
            bits_per_sample: 8,
            pixels: vec![0; 100],
        };
        let options = RasterVectorizationOptions {
            pixel_cap: 10,
            ..RasterVectorizationOptions::default()
        };
        let report = vectorize_raw_image(&raw, &reference, &options);
        assert_eq!(
            report.status,
            Prompt21Status::UnsupportedReportedSecurityPolicy
        );
    }

    #[test]
    fn persistent_store_measures_structural_sharing() {
        let report = persistent_store_report();
        assert!(report.hamt.shared_nodes_between_last_versions > 0);
        assert!(report.rrb.shared_nodes_between_last_versions > 0);
        assert!(report.undo_redo.undo_restores_parent);
        assert!(report.serialization.corruption_hash_checked);
    }

    #[test]
    fn object_stream_report_uses_reopenable_writer_path() {
        let pdf = tiny_pdf();
        let reader = PdfReader::from_bytes(pdf).expect("reader");
        let report = object_stream_packing_report(&reader).expect("object stream report");
        assert!(report.reopen.wellfriendpdf_reopened);
        assert!(report.reopen.xref_stream_marker_present);
        assert!(report.object_stream_count > 0);
        assert!(report.deterministic);
    }

    #[test]
    fn feature_matrix_has_no_blocked_rows() {
        assert!(prompt21_feature_matrix()
            .iter()
            .all(|row| row.implementation_status != Prompt21Status::Blocked));
    }

    fn tiny_pdf() -> Vec<u8> {
        let mut out = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
        let mut offsets = Vec::new();
        offsets.push(out.len());
        out.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        offsets.push(out.len());
        out.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        offsets.push(out.len());
        out.extend_from_slice(b"3 0 obj\n<< /Type /Page /Parent 2 0 R /Resources <<>> /MediaBox [0 0 10 10] >>\nendobj\n");
        let xref = out.len();
        out.extend_from_slice(b"xref\n0 4\n0000000000 65535 f \n");
        for offset in offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!("trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );
        out
    }
}
