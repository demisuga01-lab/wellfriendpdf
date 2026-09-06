//! Transaction-driven narrow dependency invalidation (RB-02).
//!
//! This module bridges editing transaction reports (which carry `affected_objects`
//! and `affected_pages`) into the render cache invalidation graph. Callers invoke
//! [`TransactionWriteSet::invalidate`] after a source edit to evict only the
//! page/tile caches whose dependencies are proven affected, while unknown object
//! references trigger a conservative full reset.

use std::collections::BTreeSet;

use serde::{de::Error as SerdeDeError, Deserialize, Deserializer, Serialize};
use serde_json::Value;

use super::contract::{ObjectIdentityId, RevisionId};
use super::display_list::RenderTile;
use super::invalidation::InvalidationResult;
use super::page_renderer::{source_cache_markers_for_object, RenderDocumentCache};
use super::transform::Viewport;
use crate::error::{Result, WellfriendError};
use crate::render::document_view::ObjectIdentity;

pub const RENDER_TRANSACTION_INVALIDATION_PLAN_SCHEMA_VERSION: &str =
    "render-transaction-invalidation-plan.v1";
const RENDER_WRITE_SET_MUTATION_PREFIXES: &[&str] = &[
    "created", "removed", "deleted", "added", "updated", "modified", "replaced",
];

/// A typed write-set produced by an editing transaction, suitable for driving
/// narrow cache invalidation. Callers build this from the transaction report's
/// `affected_objects` and `affected_pages`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransactionWriteSet {
    /// Object reference strings from the transaction report (e.g. "4 0 R").
    #[serde(alias = "affectedObjectRefs", alias = "affectedObjects")]
    pub affected_object_refs: Vec<String>,
    /// Page numbers the transaction reported as affected.
    #[serde(alias = "affectedPages")]
    pub affected_pages: Vec<usize>,
    /// Optional pixel-space page tiles the transaction has proven dirty.
    #[serde(
        default,
        alias = "affectedTiles",
        deserialize_with = "deserialize_transaction_affected_tiles"
    )]
    pub affected_tiles: Vec<(usize, RenderTile)>,
    /// Next document revision (derived from output bytes hash).
    #[serde(alias = "nextRevision")]
    pub next_revision: RevisionId,
}

/// Result of applying a transaction write-set to the render cache.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransactionInvalidationResult {
    /// The underlying invalidation result from the dependency graph.
    #[serde(alias = "invalidationResult")]
    pub invalidation: InvalidationResult,
    /// Object refs that could not be mapped to canonical IDs (triggers conservative reset).
    #[serde(alias = "unmappedRefs")]
    pub unmapped_refs: Vec<String>,
    /// Object refs that were successfully mapped.
    #[serde(alias = "mappedIds")]
    pub mapped_ids: Vec<ObjectIdentityId>,
}

fn deserialize_transaction_affected_tiles<'de, D>(
    deserializer: D,
) -> std::result::Result<Vec<(usize, RenderTile)>, D::Error>
where
    D: Deserializer<'de>,
{
    let values = Vec::<Value>::deserialize(deserializer)?;
    let mut tiles = Vec::with_capacity(values.len());
    for value in values {
        if let Ok((page, tile)) = serde_json::from_value::<(usize, RenderTile)>(value.clone()) {
            tiles.push((page, tile));
            continue;
        }
        if let Ok(entry) = serde_json::from_value::<RenderInvalidationPlanTile>(value.clone()) {
            tiles.push((entry.page, entry.tile));
            continue;
        }
        return Err(D::Error::custom(
            "affected_tiles entries must be [page, tile] tuples or {page, tile} objects",
        ));
    }
    Ok(tiles)
}

/// Binding/server-safe tile entry from a render-invalidation plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderInvalidationPlanTile {
    #[serde(alias = "pageNumber")]
    pub page: usize,
    pub tile: RenderTile,
}

/// Source-bound artifact cache markers for a mapped source object.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderInvalidationSourceCacheMarkers {
    #[serde(alias = "sourceId")]
    pub source_id: ObjectIdentityId,
    #[serde(alias = "objectNumber")]
    pub object_number: u32,
    #[serde(alias = "generationNumber", alias = "objectGeneration")]
    pub generation: u16,
    #[serde(default)]
    pub markers: Vec<String>,
}

/// Cache-application subset of a binding-safe render-invalidation plan.
///
/// The SDK report is intentionally richer than this structure. Cache owners only
/// need the revision, mapped source IDs, source cache markers, affected pages,
/// exact dirty tiles, and conservative-reset bit to mutate their local
/// [`RenderDocumentCache`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderInvalidationCachePlan {
    #[serde(alias = "schemaVersion")]
    pub schema_version: String,
    #[serde(alias = "nextRevision")]
    pub next_revision: RevisionId,
    #[serde(default, alias = "affectedPages")]
    pub affected_pages: Vec<usize>,
    #[serde(default, alias = "mappedSourceIds")]
    pub mapped_source_ids: Vec<ObjectIdentityId>,
    #[serde(default, alias = "sourceCacheMarkers")]
    pub source_cache_markers: Vec<RenderInvalidationSourceCacheMarkers>,
    #[serde(default, alias = "affectedTiles")]
    pub affected_tiles: Vec<RenderInvalidationPlanTile>,
    #[serde(default, alias = "conservativeResetRequired")]
    pub conservative_reset_required: bool,
}

impl RenderInvalidationCachePlan {
    pub fn from_json(plan_json: &str) -> Result<Self> {
        let value = serde_json::from_str::<Value>(plan_json).map_err(|err| {
            WellfriendError::invalid_input(format!(
                "render invalidation plan JSON parse error: {err}"
            ))
        })?;
        let plan_value = extract_render_invalidation_plan_value(&value)
            .cloned()
            .ok_or_else(|| {
                WellfriendError::invalid_input(
                    "render invalidation plan JSON must be a plan object or SDK envelope report",
                )
            })?;
        let plan = serde_json::from_value::<Self>(plan_value).map_err(|err| {
            WellfriendError::invalid_input(format!(
                "render invalidation plan JSON decode error: {err}"
            ))
        })?;
        if plan.schema_version != RENDER_TRANSACTION_INVALIDATION_PLAN_SCHEMA_VERSION {
            return Err(WellfriendError::invalid_input(format!(
                "unsupported render invalidation plan schema_version '{}'",
                plan.schema_version
            )));
        }
        Ok(plan)
    }

    pub fn affected_tile_tuples(&self) -> Vec<(usize, RenderTile)> {
        valid_affected_tiles(
            &self
                .affected_tiles
                .iter()
                .map(|entry| (entry.page, entry.tile))
                .collect::<Vec<_>>(),
        )
    }

    /// Merge page-space dirty regions into this plan's exact affected-tile set.
    ///
    /// The plan deliberately does not infer page geometry. Cache owners that
    /// know the active render viewport and tile grid can call this before
    /// [`Self::apply_to_cache`] to narrow a page-level dirty-region report to
    /// exact raster tiles.
    pub fn merge_dirty_region_tiles(
        &mut self,
        dirty_regions: &[Value],
        page_number: usize,
        viewport: &Viewport,
        tile_width: u32,
        tile_height: u32,
    ) -> usize {
        let mut added = 0;
        for (page, tile) in dirty_regions_to_render_tiles(
            dirty_regions,
            page_number,
            viewport,
            tile_width,
            tile_height,
        ) {
            if self
                .affected_tiles
                .iter()
                .any(|entry| entry.page == page && entry.tile == tile)
            {
                continue;
            }
            self.affected_tiles
                .push(RenderInvalidationPlanTile { page, tile });
            added += 1;
        }
        added
    }

    pub fn apply_to_cache(&self, cache: &mut RenderDocumentCache) -> InvalidationResult {
        if self.conservative_reset_required {
            return cache.reset_to_revision_with_result(self.next_revision);
        }
        for source_markers in &self.source_cache_markers {
            let markers = if source_markers.markers.is_empty() {
                source_cache_markers_for_object(
                    source_markers.object_number,
                    source_markers.generation,
                )
            } else {
                source_markers.markers.clone()
            };
            cache.remember_source_cache_markers(source_markers.source_id, markers);
        }
        let affected_tiles = self.affected_tile_tuples();
        if !exact_tiles_cover_all_reported_pages(&self.affected_pages, &affected_tiles) {
            cache.invalidate_sources_pages_and_tiles(
                self.next_revision,
                &self.mapped_source_ids,
                &self.affected_pages,
                &affected_tiles,
            )
        } else {
            cache.invalidate_sources_page_artifacts_and_exact_tiles(
                self.next_revision,
                &self.mapped_source_ids,
                &self.affected_pages,
                &affected_tiles,
            )
        }
    }

    fn merge_mapped_source_id(&mut self, source_id: ObjectIdentityId) {
        if !self.mapped_source_ids.contains(&source_id) {
            self.mapped_source_ids.push(source_id);
        }
    }

    fn merge_source_cache_marker_entry(
        &mut self,
        source_id: ObjectIdentityId,
        object_number: u32,
        generation: u16,
    ) {
        if self
            .source_cache_markers
            .iter()
            .any(|entry| entry.source_id == source_id)
        {
            return;
        }
        self.source_cache_markers
            .push(RenderInvalidationSourceCacheMarkers {
                source_id,
                object_number,
                generation,
                markers: Vec::new(),
            });
    }

    fn merge_nested_write_set_refs(
        &mut self,
        cache: &RenderDocumentCache,
        refs: &[String],
    ) -> Vec<String> {
        let mut unmapped = Vec::new();
        for ref_string in refs {
            let Some((number, generation)) = parse_changed_object_ref(ref_string) else {
                if !unmapped.contains(ref_string) {
                    unmapped.push(ref_string.clone());
                }
                continue;
            };
            if let Some(source_id) = cache.source_identity_for_object(number, generation) {
                self.merge_mapped_source_id(source_id);
                self.merge_source_cache_marker_entry(source_id, number, generation);
            } else if !unmapped.contains(ref_string) {
                unmapped.push(ref_string.clone());
            }
        }
        unmapped
    }
}

pub fn apply_render_invalidation_plan_json_to_cache(
    cache: &mut RenderDocumentCache,
    plan_json: &str,
) -> Result<InvalidationResult> {
    let value = serde_json::from_str::<Value>(plan_json).map_err(|err| {
        WellfriendError::invalid_input(format!("render invalidation plan JSON parse error: {err}"))
    })?;
    let plan_value = extract_render_invalidation_plan_value(&value)
        .cloned()
        .ok_or_else(|| {
            WellfriendError::invalid_input(
                "render invalidation plan JSON must be a plan object or SDK envelope report",
            )
        })?;
    let mut plan =
        serde_json::from_value::<RenderInvalidationCachePlan>(plan_value).map_err(|err| {
            WellfriendError::invalid_input(format!(
                "render invalidation plan JSON decode error: {err}"
            ))
        })?;
    if plan.schema_version != RENDER_TRANSACTION_INVALIDATION_PLAN_SCHEMA_VERSION {
        return Err(WellfriendError::invalid_input(format!(
            "unsupported render invalidation plan schema_version '{}'",
            plan.schema_version
        )));
    }
    let nested_refs = collect_render_write_set_refs(&value);
    if !nested_refs.is_empty() {
        let unmapped = plan.merge_nested_write_set_refs(cache, &nested_refs);
        if !unmapped.is_empty() {
            plan.conservative_reset_required = true;
        }
    }
    Ok(plan.apply_to_cache(cache))
}

fn extract_render_invalidation_plan_value(value: &Value) -> Option<&Value> {
    let Value::Object(map) = value else {
        return None;
    };
    if first_string_by_alias(map, &["schema_version"])
        == Some(RENDER_TRANSACTION_INVALIDATION_PLAN_SCHEMA_VERSION)
    {
        return Some(value);
    }
    if let Some(plan) = first_value_by_alias(map, "render_invalidation") {
        return Some(plan);
    }
    first_value_by_alias(map, "report").and_then(|report| {
        let Value::Object(report) = report else {
            return None;
        };
        first_value_by_alias(report, "render_invalidation")
    })
}

fn collect_render_write_set_refs(value: &Value) -> Vec<String> {
    const BROAD_REF_FIELDS: &[&str] = &[
        "affected_objects",
        "write_set",
        "dirty_objects",
        "source_refs",
        "object_refs",
    ];
    const REF_FIELDS: &[&str] = &[
        "render_write_set_refs",
        "changed_object_refs",
        "created_object_refs",
        "removed_object_refs",
        "changed_stream_refs",
        "changed_content_stream_refs",
        "changed_page_content_refs",
        "changed_page_program_refs",
        "changed_retained_operation_refs",
        "changed_retained_ops_refs",
        "changed_display_operation_refs",
        "changed_display_op_refs",
        "changed_display_list_refs",
        "changed_render_resource_refs",
        "changed_render_sublist_refs",
        "changed_form_sublist_refs",
        "changed_type3_sublist_refs",
        "changed_pattern_sublist_refs",
        "changed_appearance_sublist_refs",
        "changed_spatial_entry_refs",
        "changed_spatial_index_refs",
        "changed_backend_plan_refs",
        "changed_backend_plan_cache_refs",
        "changed_render_cache_refs",
        "changed_cache_entry_refs",
        "changed_tile_refs",
        "changed_render_tile_refs",
        "changed_page_refs",
        "changed_page_object_refs",
        "changed_page_tree_refs",
        "changed_pages_tree_refs",
        "changed_catalog_refs",
        "changed_document_catalog_refs",
        "changed_metadata_refs",
        "changed_xmp_metadata_refs",
        "changed_names_refs",
        "changed_name_tree_refs",
        "changed_page_label_refs",
        "changed_struct_tree_refs",
        "changed_structure_tree_refs",
        "changed_parent_tree_refs",
        "changed_role_map_refs",
        "changed_resource_refs",
        "changed_resource_dictionary_refs",
        "changed_page_resource_refs",
        "changed_page_resource_dictionary_refs",
        "changed_font_refs",
        "changed_font_descriptor_refs",
        "changed_cmap_refs",
        "changed_type3_refs",
        "changed_type3_glyph_refs",
        "changed_charproc_refs",
        "changed_xobject_refs",
        "changed_image_refs",
        "changed_image_filter_refs",
        "changed_filter_refs",
        "changed_decode_refs",
        "changed_decode_array_refs",
        "changed_decode_params_refs",
        "changed_decodeparms_refs",
        "changed_source_region_refs",
        "changed_image_region_refs",
        "changed_region_decode_refs",
        "changed_image_reduction_refs",
        "changed_reduction_refs",
        "changed_image_component_refs",
        "changed_component_selection_refs",
        "changed_codec_tile_refs",
        "changed_image_tile_refs",
        "changed_jpx_tile_refs",
        "changed_jpx_component_refs",
        "changed_progressive_image_refs",
        "changed_image_progression_refs",
        "changed_interpolate_refs",
        "changed_interpolation_refs",
        "changed_image_mask_refs",
        "changed_color_key_mask_refs",
        "changed_image_color_key_mask_refs",
        "changed_mask_matte_refs",
        "changed_image_matte_refs",
        "changed_smask_matte_refs",
        "changed_jpeg_params_refs",
        "changed_dct_params_refs",
        "changed_jpx_params_refs",
        "changed_ccitt_params_refs",
        "changed_jbig2_globals_refs",
        "changed_color_space_refs",
        "changed_colorspace_refs",
        "changed_ext_gstate_refs",
        "changed_graphics_state_refs",
        "changed_form_refs",
        "changed_form_xobject_refs",
        "changed_form_field_refs",
        "changed_field_refs",
        "changed_form_value_refs",
        "changed_acroform_refs",
        "changed_pattern_refs",
        "changed_tiling_pattern_refs",
        "changed_shading_refs",
        "changed_ap_refs",
        "changed_appearance_refs",
        "changed_annotation_ap_refs",
        "changed_annotation_appearance_refs",
        "changed_widget_ap_refs",
        "changed_widget_appearance_refs",
        "changed_form_ap_refs",
        "changed_form_appearance_refs",
        "changed_signature_ap_refs",
        "changed_signature_appearance_refs",
        "changed_ap_stream_refs",
        "changed_appearance_stream_refs",
        "changed_annotation_ap_stream_refs",
        "changed_widget_ap_stream_refs",
        "changed_normal_appearance_refs",
        "changed_rollover_appearance_refs",
        "changed_down_appearance_refs",
        "changed_normal_ap_refs",
        "changed_rollover_ap_refs",
        "changed_down_ap_refs",
        "changed_normal_appearance_stream_refs",
        "changed_rollover_appearance_stream_refs",
        "changed_down_appearance_stream_refs",
        "changed_normal_ap_stream_refs",
        "changed_rollover_ap_stream_refs",
        "changed_down_ap_stream_refs",
        "changed_appearance_state_refs",
        "changed_appearance_state_stream_refs",
        "changed_annotation_appearance_state_refs",
        "changed_widget_appearance_state_refs",
        "before_appearance_refs",
        "after_appearance_refs",
        "before_widget_appearance_refs",
        "after_widget_appearance_refs",
        "before_annotation_appearance_refs",
        "after_annotation_appearance_refs",
        "changed_mask_refs",
        "changed_soft_mask_refs",
        "changed_image_smask_refs",
        "changed_smask_refs",
        "changed_group_refs",
        "changed_transparency_group_refs",
        "changed_group_xobject_refs",
        "changed_optional_content_refs",
        "changed_optional_content_state_refs",
        "changed_optional_content_config_refs",
        "changed_oc_refs",
        "changed_ocg_refs",
        "changed_ocmd_refs",
        "changed_oc_config_refs",
        "changed_oc_properties_refs",
        "changed_properties_refs",
        "changed_annotation_refs",
        "changed_widget_refs",
        "changed_render_structure_refs",
        "changed_render_relevant_structure_refs",
        "changed_output_intent_refs",
        "changed_display_output_intent_refs",
        "changed_print_output_intent_refs",
        "changed_proof_output_intent_refs",
        "changed_icc_profile_refs",
        "changed_display_profile_refs",
        "changed_proof_profile_refs",
        "changed_proofing_profile_refs",
        "changed_color_management_profile_refs",
        "changed_color_management_policy_refs",
        "changed_cmm_profile_refs",
        "changed_separation_profile_refs",
        "changed_devicen_profile_refs",
        "changed_device_n_profile_refs",
        "changed_rendering_intent_refs",
        "changed_transfer_function_refs",
        "changed_halftone_refs",
        "changed_halftone_screen_refs",
        "changed_overprint_refs",
        "changed_overprint_state_refs",
        "changed_overprint_mode_refs",
        "changed_prepress_refs",
        "changed_prepress_policy_refs",
        "changed_prepress_plate_refs",
        "changed_plate_refs",
        "changed_ink_refs",
        "changed_spot_color_refs",
        "changed_spot_plate_refs",
        "changed_separation_plate_refs",
        "changed_devicen_plate_refs",
        "changed_device_n_plate_refs",
        "changed_black_point_compensation_refs",
        "changed_black_generation_refs",
        "changed_undercolor_removal_refs",
        "changed_trapping_refs",
        "changed_trap_network_refs",
        "changed_print_profile_refs",
        "changed_render_contract_refs",
        "shared_resource_refs",
        "resource_write_set_refs",
        "transitive_write_set_refs",
        "field_widget_refs",
        "ap_write_set_refs",
    ];
    let mut refs = BTreeSet::new();
    collect_render_write_set_refs_inner(value, &mut refs, REF_FIELDS, BROAD_REF_FIELDS);
    refs.into_iter().collect()
}

fn collect_render_write_set_refs_inner(
    value: &Value,
    refs: &mut BTreeSet<String>,
    ref_fields: &[&str],
    broad_ref_fields: &[&str],
) {
    match value {
        Value::Object(map) => {
            for (key, item) in map {
                if ref_fields
                    .iter()
                    .any(|field| render_write_set_field_matches(key, field))
                {
                    collect_ref_values(item, refs);
                } else if broad_ref_fields
                    .iter()
                    .any(|field| render_write_set_field_matches(key, field))
                {
                    collect_parseable_ref_values(item, refs);
                }
                collect_render_write_set_refs_inner(item, refs, ref_fields, broad_ref_fields);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_render_write_set_refs_inner(item, refs, ref_fields, broad_ref_fields);
            }
        }
        _ => {}
    }
}

fn collect_ref_values(value: &Value, refs: &mut BTreeSet<String>) {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                refs.insert(trimmed.to_string());
            }
        }
        Value::Array(items) => {
            if let Some((number, generation)) = structured_ref_array(items) {
                refs.insert(format!("{number} {generation} R"));
            } else {
                for item in items {
                    collect_ref_values(item, refs);
                }
            }
        }
        Value::Object(map) => {
            if let Some((number, generation)) = structured_ref_object(map) {
                refs.insert(format!("{number} {generation} R"));
            } else {
                for item in map.values() {
                    collect_ref_values(item, refs);
                }
            }
        }
        _ => {}
    }
}

fn collect_parseable_ref_values(value: &Value, refs: &mut BTreeSet<String>) {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            if let Some((number, generation)) = parse_changed_object_ref(trimmed) {
                refs.insert(format!("{number} {generation} R"));
            }
        }
        Value::Array(items) => {
            if let Some((number, generation)) = structured_ref_array(items) {
                refs.insert(format!("{number} {generation} R"));
            } else {
                for item in items {
                    collect_parseable_ref_values(item, refs);
                }
            }
        }
        Value::Object(map) => {
            if let Some((number, generation)) = structured_ref_object(map) {
                refs.insert(format!("{number} {generation} R"));
            } else {
                for item in map.values() {
                    collect_parseable_ref_values(item, refs);
                }
            }
        }
        _ => {}
    }
}

fn structured_ref_array(items: &[Value]) -> Option<(u32, u16)> {
    if items.len() != 2 {
        return None;
    }
    Some((value_as_u32(&items[0])?, value_as_u16(&items[1])?))
}

fn structured_ref_object(map: &serde_json::Map<String, Value>) -> Option<(u32, u16)> {
    if let Some(ref_text) = first_string_by_alias(map, &["ref", "object_ref", "source_ref"])
        .and_then(parse_changed_object_ref)
    {
        return Some(ref_text);
    }
    let number = first_numeric_u32(
        map,
        &[
            "object_number",
            "number",
            "object",
            "target_object",
            "source_object",
            "stream_object",
            "content_stream_object",
            "page_content_object",
            "page_program_object",
            "retained_operation_object",
            "retained_ops_object",
            "display_operation_object",
            "display_op_object",
            "display_list_object",
            "render_resource_object",
            "render_sublist_object",
            "form_sublist_object",
            "type3_sublist_object",
            "pattern_sublist_object",
            "appearance_sublist_object",
            "spatial_entry_object",
            "spatial_index_object",
            "backend_plan_object",
            "backend_plan_cache_object",
            "render_cache_object",
            "cache_entry_object",
            "tile_object",
            "render_tile_object",
            "page_object",
            "page_tree_object",
            "pages_tree_object",
            "catalog_object",
            "document_catalog_object",
            "metadata_object",
            "xmp_metadata_object",
            "names_object",
            "name_tree_object",
            "page_label_object",
            "struct_tree_object",
            "structure_tree_object",
            "parent_tree_object",
            "role_map_object",
            "resource_object",
            "resource_dictionary_object",
            "page_resource_object",
            "page_resource_dictionary_object",
            "form_object",
            "form_xobject_object",
            "xobject_object",
            "image_object",
            "image_filter_object",
            "filter_object",
            "decode_object",
            "decode_array_object",
            "decode_params_object",
            "decodeparms_object",
            "source_region_object",
            "image_region_object",
            "region_decode_object",
            "image_reduction_object",
            "reduction_object",
            "image_component_object",
            "component_selection_object",
            "codec_tile_object",
            "image_tile_object",
            "jpx_tile_object",
            "jpx_component_object",
            "progressive_image_object",
            "image_progression_object",
            "interpolate_object",
            "interpolation_object",
            "image_mask_object",
            "color_key_mask_object",
            "image_color_key_mask_object",
            "mask_matte_object",
            "image_matte_object",
            "smask_matte_object",
            "jpeg_params_object",
            "dct_params_object",
            "jpx_params_object",
            "ccitt_params_object",
            "jbig2_globals_object",
            "font_object",
            "font_descriptor_object",
            "cmap_object",
            "color_space_object",
            "colorspace_object",
            "ext_gstate_object",
            "graphics_state_object",
            "pattern_object",
            "tiling_pattern_object",
            "shading_object",
            "type3_object",
            "type3_glyph_object",
            "charproc_object",
            "annotation_object",
            "annotation_ap_object",
            "annotation_appearance_object",
            "widget_ap_object",
            "widget_appearance_object",
            "form_ap_object",
            "form_appearance_object",
            "signature_ap_object",
            "signature_appearance_object",
            "ap_object",
            "ap_stream_object",
            "appearance_object",
            "appearance_stream_object",
            "annotation_ap_stream_object",
            "widget_ap_stream_object",
            "normal_appearance_object",
            "rollover_appearance_object",
            "down_appearance_object",
            "normal_ap_object",
            "rollover_ap_object",
            "down_ap_object",
            "normal_appearance_stream_object",
            "rollover_appearance_stream_object",
            "down_appearance_stream_object",
            "normal_ap_stream_object",
            "rollover_ap_stream_object",
            "down_ap_stream_object",
            "appearance_state_object",
            "appearance_state_stream_object",
            "annotation_appearance_state_object",
            "widget_appearance_state_object",
            "mask_object",
            "soft_mask_object",
            "image_smask_object",
            "smask_object",
            "group_object",
            "transparency_group_object",
            "group_xobject_object",
            "optional_content_object",
            "optional_content_state_object",
            "optional_content_config_object",
            "oc_object",
            "ocg_object",
            "ocmd_object",
            "oc_config_object",
            "oc_properties_object",
            "properties_object",
            "widget_object",
            "field_object",
            "form_field_object",
            "form_value_object",
            "acroform_object",
            "render_structure_object",
            "render_relevant_structure_object",
            "output_intent_object",
            "display_output_intent_object",
            "print_output_intent_object",
            "proof_output_intent_object",
            "icc_profile_object",
            "display_profile_object",
            "proof_profile_object",
            "proofing_profile_object",
            "color_management_profile_object",
            "color_management_policy_object",
            "cmm_profile_object",
            "separation_profile_object",
            "devicen_profile_object",
            "device_n_profile_object",
            "rendering_intent_object",
            "transfer_function_object",
            "halftone_object",
            "halftone_screen_object",
            "overprint_object",
            "overprint_state_object",
            "overprint_mode_object",
            "prepress_object",
            "prepress_policy_object",
            "prepress_plate_object",
            "plate_object",
            "ink_object",
            "spot_color_object",
            "spot_plate_object",
            "separation_plate_object",
            "devicen_plate_object",
            "device_n_plate_object",
            "black_point_compensation_object",
            "black_generation_object",
            "undercolor_removal_object",
            "trapping_object",
            "trap_network_object",
            "print_profile_object",
            "render_contract_object",
        ],
    )?;
    let generation = first_numeric_u16(
        map,
        &[
            "generation",
            "generation_number",
            "object_generation",
            "target_generation",
            "source_generation",
            "stream_generation",
            "content_stream_generation",
            "page_content_generation",
            "page_program_generation",
            "retained_operation_generation",
            "retained_ops_generation",
            "display_operation_generation",
            "display_op_generation",
            "display_list_generation",
            "render_resource_generation",
            "render_sublist_generation",
            "form_sublist_generation",
            "type3_sublist_generation",
            "pattern_sublist_generation",
            "appearance_sublist_generation",
            "spatial_entry_generation",
            "spatial_index_generation",
            "backend_plan_generation",
            "backend_plan_cache_generation",
            "render_cache_generation",
            "cache_entry_generation",
            "tile_generation",
            "render_tile_generation",
            "page_generation",
            "page_tree_generation",
            "pages_tree_generation",
            "catalog_generation",
            "document_catalog_generation",
            "metadata_generation",
            "xmp_metadata_generation",
            "names_generation",
            "name_tree_generation",
            "page_label_generation",
            "struct_tree_generation",
            "structure_tree_generation",
            "parent_tree_generation",
            "role_map_generation",
            "resource_generation",
            "resource_dictionary_generation",
            "page_resource_generation",
            "page_resource_dictionary_generation",
            "form_generation",
            "form_xobject_generation",
            "xobject_generation",
            "image_generation",
            "image_filter_generation",
            "filter_generation",
            "decode_generation",
            "decode_array_generation",
            "decode_params_generation",
            "decodeparms_generation",
            "source_region_generation",
            "image_region_generation",
            "region_decode_generation",
            "image_reduction_generation",
            "reduction_generation",
            "image_component_generation",
            "component_selection_generation",
            "codec_tile_generation",
            "image_tile_generation",
            "jpx_tile_generation",
            "jpx_component_generation",
            "progressive_image_generation",
            "image_progression_generation",
            "interpolate_generation",
            "interpolation_generation",
            "image_mask_generation",
            "color_key_mask_generation",
            "image_color_key_mask_generation",
            "mask_matte_generation",
            "image_matte_generation",
            "smask_matte_generation",
            "jpeg_params_generation",
            "dct_params_generation",
            "jpx_params_generation",
            "ccitt_params_generation",
            "jbig2_globals_generation",
            "font_generation",
            "font_descriptor_generation",
            "cmap_generation",
            "color_space_generation",
            "colorspace_generation",
            "ext_gstate_generation",
            "graphics_state_generation",
            "pattern_generation",
            "tiling_pattern_generation",
            "shading_generation",
            "type3_generation",
            "type3_glyph_generation",
            "charproc_generation",
            "annotation_generation",
            "annotation_ap_generation",
            "annotation_appearance_generation",
            "widget_ap_generation",
            "widget_appearance_generation",
            "form_ap_generation",
            "form_appearance_generation",
            "signature_ap_generation",
            "signature_appearance_generation",
            "ap_generation",
            "ap_stream_generation",
            "appearance_generation",
            "appearance_stream_generation",
            "annotation_ap_stream_generation",
            "widget_ap_stream_generation",
            "normal_appearance_generation",
            "rollover_appearance_generation",
            "down_appearance_generation",
            "normal_ap_generation",
            "rollover_ap_generation",
            "down_ap_generation",
            "normal_appearance_stream_generation",
            "rollover_appearance_stream_generation",
            "down_appearance_stream_generation",
            "normal_ap_stream_generation",
            "rollover_ap_stream_generation",
            "down_ap_stream_generation",
            "appearance_state_generation",
            "appearance_state_stream_generation",
            "annotation_appearance_state_generation",
            "widget_appearance_state_generation",
            "mask_generation",
            "soft_mask_generation",
            "image_smask_generation",
            "smask_generation",
            "group_generation",
            "transparency_group_generation",
            "group_xobject_generation",
            "optional_content_generation",
            "optional_content_state_generation",
            "optional_content_config_generation",
            "oc_generation",
            "ocg_generation",
            "ocmd_generation",
            "oc_config_generation",
            "oc_properties_generation",
            "properties_generation",
            "widget_generation",
            "field_generation",
            "form_field_generation",
            "form_value_generation",
            "acroform_generation",
            "render_structure_generation",
            "render_relevant_structure_generation",
            "output_intent_generation",
            "display_output_intent_generation",
            "print_output_intent_generation",
            "proof_output_intent_generation",
            "icc_profile_generation",
            "display_profile_generation",
            "proof_profile_generation",
            "proofing_profile_generation",
            "color_management_profile_generation",
            "color_management_policy_generation",
            "cmm_profile_generation",
            "separation_profile_generation",
            "devicen_profile_generation",
            "device_n_profile_generation",
            "rendering_intent_generation",
            "transfer_function_generation",
            "halftone_generation",
            "halftone_screen_generation",
            "overprint_generation",
            "overprint_state_generation",
            "overprint_mode_generation",
            "prepress_generation",
            "prepress_policy_generation",
            "prepress_plate_generation",
            "plate_generation",
            "ink_generation",
            "spot_color_generation",
            "spot_plate_generation",
            "separation_plate_generation",
            "devicen_plate_generation",
            "device_n_plate_generation",
            "black_point_compensation_generation",
            "black_generation_generation",
            "black_generation",
            "undercolor_removal_generation",
            "trapping_generation",
            "trap_network_generation",
            "print_profile_generation",
            "render_contract_generation",
        ],
    )?;
    Some((number, generation))
}

fn first_numeric_u32(map: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<u32> {
    keys.iter()
        .find_map(|key| first_ref_component_value_by_alias(map, key).and_then(value_as_u32))
}

fn first_numeric_u16(map: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<u16> {
    keys.iter()
        .find_map(|key| first_ref_component_value_by_alias(map, key).and_then(value_as_u16))
}

fn first_string_by_alias<'a>(
    map: &'a serde_json::Map<String, Value>,
    keys: &[&str],
) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| first_value_by_alias(map, key).and_then(Value::as_str))
}

fn first_value_by_alias<'a>(
    map: &'a serde_json::Map<String, Value>,
    expected: &str,
) -> Option<&'a Value> {
    map.iter()
        .find_map(|(key, value)| json_field_key_matches(key, expected).then_some(value))
}

fn first_ref_component_value_by_alias<'a>(
    map: &'a serde_json::Map<String, Value>,
    expected: &str,
) -> Option<&'a Value> {
    map.iter()
        .find_map(|(key, value)| json_ref_component_key_matches(key, expected).then_some(value))
}

fn json_field_key_matches(actual: &str, expected: &str) -> bool {
    actual == expected || normalize_json_field_key(actual) == normalize_json_field_key(expected)
}

fn render_write_set_field_matches(actual: &str, expected: &str) -> bool {
    if json_field_key_matches(actual, expected) {
        return true;
    }

    let actual = normalize_json_field_key(actual);
    let expected = normalize_json_field_key(expected);
    let Some(expected_suffix) = expected.strip_prefix("changed") else {
        return false;
    };

    RENDER_WRITE_SET_MUTATION_PREFIXES
        .iter()
        .filter_map(|prefix| actual.strip_prefix(prefix))
        .any(|actual_suffix| actual_suffix == expected_suffix)
}

fn json_ref_component_key_matches(actual: &str, expected: &str) -> bool {
    if json_field_key_matches(actual, expected) {
        return true;
    }

    let actual = normalize_json_field_key(actual);
    let expected = normalize_json_field_key(expected);
    RENDER_WRITE_SET_MUTATION_PREFIXES
        .iter()
        .filter_map(|prefix| actual.strip_prefix(prefix))
        .any(|actual_suffix| actual_suffix == expected)
}

fn normalize_json_field_key(key: &str) -> String {
    key.bytes()
        .filter(|byte| !matches!(*byte, b'_' | b'-'))
        .map(|byte| (byte as char).to_ascii_lowercase())
        .collect()
}

fn value_as_u32(value: &Value) -> Option<u32> {
    u32::try_from(value.as_u64()?).ok()
}

fn value_as_u16(value: &Value) -> Option<u16> {
    u16::try_from(value.as_u64()?).ok()
}

fn value_as_usize(value: &Value) -> Option<usize> {
    usize::try_from(value.as_u64()?).ok()
}

/// Parse a PDF object reference string like "4 0 R" into (number, generation).
fn parse_object_ref(ref_str: &str) -> Option<(u32, u16)> {
    let parts: Vec<&str> = ref_str.split_whitespace().collect();
    if parts.len() >= 2 {
        let number = parts[0].parse::<u32>().ok()?;
        let generation = parts[1].parse::<u16>().ok()?;
        Some((number, generation))
    } else {
        None
    }
}

/// Parse source-editing identities like "object-4-0-revision-..." or
/// "stream-4-0-revision-..." into (number, generation).
fn parse_source_identity_ref(ref_str: &str) -> Option<(u32, u16)> {
    let tail = ref_str
        .strip_prefix("object-")
        .or_else(|| ref_str.strip_prefix("stream-"))?;
    let mut parts = tail.splitn(3, '-');
    let number = parts.next()?.parse::<u32>().ok()?;
    let generation = parts.next()?.parse::<u16>().ok()?;
    Some((number, generation))
}

fn parse_changed_object_ref(ref_str: &str) -> Option<(u32, u16)> {
    parse_object_ref(ref_str).or_else(|| parse_source_identity_ref(ref_str))
}

fn parse_dirty_region_value(value: &Value) -> Option<(usize, [f64; 4])> {
    let map = value.as_object()?;
    let page_number = first_value_by_alias(map, "page")
        .or_else(|| first_value_by_alias(map, "page_number"))
        .or_else(|| first_value_by_alias(map, "page_index"))
        .and_then(value_as_usize)?;
    let region = first_value_by_alias(map, "region")
        .or_else(|| first_value_by_alias(map, "dirty_region"))
        .or_else(|| first_value_by_alias(map, "dirty_rect"))
        .or_else(|| first_value_by_alias(map, "bounds"))
        .or_else(|| first_value_by_alias(map, "dirty_bounds"))
        .or_else(|| first_value_by_alias(map, "annotation_rect"))
        .or_else(|| first_value_by_alias(map, "annotation_bounds"))
        .or_else(|| first_value_by_alias(map, "widget_rect"))
        .or_else(|| first_value_by_alias(map, "widget_bounds"))
        .or_else(|| first_value_by_alias(map, "appearance_rect"))
        .or_else(|| first_value_by_alias(map, "appearance_bounds"))
        .or_else(|| first_value_by_alias(map, "before_rect"))
        .or_else(|| first_value_by_alias(map, "before_bounds"))
        .or_else(|| first_value_by_alias(map, "before_region"))
        .or_else(|| first_value_by_alias(map, "after_rect"))
        .or_else(|| first_value_by_alias(map, "after_bounds"))
        .or_else(|| first_value_by_alias(map, "after_region"))
        .or_else(|| first_value_by_alias(map, "rect"))?
        .as_array()?;
    if region.len() != 4 {
        return None;
    }
    let mut bounds = [0.0; 4];
    for (index, item) in region.iter().enumerate() {
        let coordinate = item.as_f64()?;
        if !coordinate.is_finite() {
            return None;
        }
        bounds[index] = coordinate;
    }
    Some((page_number, bounds))
}

fn dirty_region_bounds_to_pixel_window(
    viewport: &Viewport,
    region: [f64; 4],
) -> Option<(u32, u32, u32, u32)> {
    let [x0, y0, x1, y1] = region;
    let user_x0 = x0.min(x1);
    let user_x1 = x0.max(x1);
    let user_y0 = y0.min(y1);
    let user_y1 = y0.max(y1);
    if user_x1 <= user_x0 || user_y1 <= user_y0 {
        return None;
    }
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for (x, y) in [
        (user_x0, user_y0),
        (user_x1, user_y0),
        (user_x0, user_y1),
        (user_x1, user_y1),
    ] {
        let (px, py) = viewport.page_to_pixel_f64(x, y);
        if !px.is_finite() || !py.is_finite() {
            return None;
        }
        min_x = min_x.min(px);
        min_y = min_y.min(py);
        max_x = max_x.max(px);
        max_y = max_y.max(py);
    }
    let left = min_x.floor().max(0.0).min(viewport.width_px as f64) as u32;
    let top = min_y.floor().max(0.0).min(viewport.height_px as f64) as u32;
    let right = max_x.ceil().max(0.0).min(viewport.width_px as f64) as u32;
    let bottom = max_y.ceil().max(0.0).min(viewport.height_px as f64) as u32;
    (right > left && bottom > top).then_some((left, top, right, bottom))
}

/// Convert transaction-report page-space dirty rectangles into page-pixel tile
/// coordinates for a caller-supplied render viewport and tile grid.
///
/// The helper intentionally does not infer page boxes, DPI, rotation, or tile
/// size from the edit report. Callers must pass the exact full-page viewport
/// used by their render contract, then feed the returned vector into
/// [`TransactionWriteSet::from_transaction_report_with_tiles`].
pub fn dirty_regions_to_render_tiles(
    dirty_regions: &[Value],
    page_number: usize,
    viewport: &Viewport,
    tile_width: u32,
    tile_height: u32,
) -> Vec<(usize, RenderTile)> {
    if tile_width == 0 || tile_height == 0 {
        return Vec::new();
    }
    let mut tiles = Vec::new();
    for value in dirty_regions {
        let Some((region_page, region)) = parse_dirty_region_value(value) else {
            continue;
        };
        if region_page != page_number {
            continue;
        }
        let Some((left, top, right, bottom)) =
            dirty_region_bounds_to_pixel_window(viewport, region)
        else {
            continue;
        };
        let start_x = (left / tile_width) * tile_width;
        let start_y = (top / tile_height) * tile_height;
        let mut y = start_y;
        while y < bottom {
            let mut x = start_x;
            while x < right {
                let tile = RenderTile {
                    x,
                    y,
                    width: tile_width.min(viewport.width_px.saturating_sub(x)),
                    height: tile_height.min(viewport.height_px.saturating_sub(y)),
                };
                let page_tile = (page_number, tile);
                if tile.width != 0 && tile.height != 0 && !tiles.contains(&page_tile) {
                    tiles.push(page_tile);
                }
                x = x.saturating_add(tile_width);
            }
            y = y.saturating_add(tile_height);
        }
    }
    tiles
}

/// Map object reference strings to canonical ObjectIdentityIds using the
/// document's identity table. Returns (mapped_ids, unmapped_refs).
pub fn map_refs_to_canonical_ids(
    object_refs: &[String],
    identities: &[ObjectIdentity],
) -> (Vec<ObjectIdentityId>, Vec<String>) {
    let mut mapped = Vec::new();
    let mut unmapped = Vec::new();
    for ref_str in object_refs {
        if let Some((number, generation)) = parse_changed_object_ref(ref_str) {
            if let Some(identity) = identities
                .iter()
                .find(|id| id.number == number && id.generation == generation)
            {
                if !mapped.contains(&identity.id) {
                    mapped.push(identity.id);
                }
            } else {
                if !unmapped.contains(ref_str) {
                    unmapped.push(ref_str.clone());
                }
            }
        } else if !unmapped.contains(ref_str) {
            unmapped.push(ref_str.clone());
        }
    }
    (mapped, unmapped)
}

fn exact_tiles_cover_all_reported_pages(
    affected_pages: &[usize],
    affected_tiles: &[(usize, RenderTile)],
) -> bool {
    affected_pages.iter().all(|page| {
        affected_tiles
            .iter()
            .any(|(tile_page, tile)| tile_page == page && render_tile_is_valid(*tile_page, tile))
    })
}

fn render_tile_is_valid(page: usize, tile: &RenderTile) -> bool {
    page > 0 && tile.width > 0 && tile.height > 0
}

fn valid_affected_tiles(affected_tiles: &[(usize, RenderTile)]) -> Vec<(usize, RenderTile)> {
    affected_tiles
        .iter()
        .copied()
        .filter(|(page, tile)| render_tile_is_valid(*page, tile))
        .collect()
}

impl TransactionWriteSet {
    /// Build a write-set from an editing transaction report's fields.
    pub fn from_transaction_report(
        affected_objects: &[String],
        affected_pages: &[usize],
        next_revision: RevisionId,
    ) -> Self {
        Self {
            affected_object_refs: affected_objects.to_vec(),
            affected_pages: affected_pages.to_vec(),
            affected_tiles: Vec::new(),
            next_revision,
        }
    }

    /// Build a write-set with explicit pixel-space dirty tiles. This is used by
    /// callers that have already converted edit dirty regions into render-tile
    /// coordinates; the ordinary report constructor remains conservative.
    pub fn from_transaction_report_with_tiles(
        affected_objects: &[String],
        affected_pages: &[usize],
        affected_tiles: &[(usize, RenderTile)],
        next_revision: RevisionId,
    ) -> Self {
        Self {
            affected_object_refs: affected_objects.to_vec(),
            affected_pages: affected_pages.to_vec(),
            affected_tiles: affected_tiles.to_vec(),
            next_revision,
        }
    }

    /// Apply this write-set to a render document cache, using the provided
    /// object identity table to map refs to canonical IDs.
    ///
    /// Known dependencies are narrowly evicted. If any object refs cannot be
    /// mapped (unknown dependencies), the cache performs a conservative full
    /// reset to prevent stale pixels.
    pub fn invalidate(
        &self,
        cache: &mut RenderDocumentCache,
        identities: &[ObjectIdentity],
    ) -> TransactionInvalidationResult {
        cache.remember_source_identities(identities);
        let (mapped_ids, unmapped_refs) =
            map_refs_to_canonical_ids(&self.affected_object_refs, identities);

        // If we have unmapped refs, we cannot prove what pages they affect —
        // force a conservative full cache reset.
        if !unmapped_refs.is_empty() {
            let invalidation = cache.invalidate_sources(self.next_revision, &[]);
            // The empty changed_sources with a new revision triggers cache_must_reset
            // in the dependency graph (revision changes but no pages found).
            return TransactionInvalidationResult {
                invalidation,
                unmapped_refs,
                mapped_ids,
            };
        }

        let affected_tiles = valid_affected_tiles(&self.affected_tiles);
        let invalidation =
            if !exact_tiles_cover_all_reported_pages(&self.affected_pages, &affected_tiles) {
                cache.invalidate_sources_pages_and_tiles(
                    self.next_revision,
                    &mapped_ids,
                    &self.affected_pages,
                    &affected_tiles,
                )
            } else {
                cache.invalidate_sources_page_artifacts_and_exact_tiles(
                    self.next_revision,
                    &mapped_ids,
                    &self.affected_pages,
                    &affected_tiles,
                )
            };
        TransactionInvalidationResult {
            invalidation,
            unmapped_refs,
            mapped_ids,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::display_list::RenderTile;
    use crate::render::page_renderer::RenderDocumentCache;
    use crate::render::Viewport;
    use serde_json::json;

    fn make_identities() -> Vec<ObjectIdentity> {
        vec![
            ObjectIdentity {
                id: ObjectIdentityId(1),
                number: 1,
                generation: 0,
            },
            ObjectIdentity {
                id: ObjectIdentityId(2),
                number: 2,
                generation: 0,
            },
            ObjectIdentity {
                id: ObjectIdentityId(3),
                number: 3,
                generation: 0,
            },
            ObjectIdentity {
                id: ObjectIdentityId(4),
                number: 4,
                generation: 0,
            },
            ObjectIdentity {
                id: ObjectIdentityId(5),
                number: 5,
                generation: 0,
            },
        ]
    }

    #[test]
    fn parse_object_ref_parses_standard_format() {
        assert_eq!(parse_object_ref("4 0 R"), Some((4, 0)));
        assert_eq!(parse_object_ref("12 1 R"), Some((12, 1)));
        assert_eq!(parse_object_ref("4 0"), Some((4, 0)));
    }

    #[test]
    fn parse_source_identity_ref_parses_editing_transaction_ids() {
        assert_eq!(
            parse_source_identity_ref("object-4-0-revision-deadbeef"),
            Some((4, 0))
        );
        assert_eq!(
            parse_source_identity_ref("stream-12-1-revision-cafebabe"),
            Some((12, 1))
        );
        assert_eq!(parse_source_identity_ref("instruction-abc"), None);
    }

    #[test]
    fn parse_object_ref_returns_none_for_invalid() {
        assert_eq!(parse_object_ref("abc"), None);
        assert_eq!(parse_object_ref(""), None);
        assert_eq!(parse_object_ref("4"), None);
    }

    #[test]
    fn map_refs_resolves_known_and_flags_unknown() {
        let identities = make_identities();
        let refs = vec![
            "4 0 R".to_string(),
            "99 0 R".to_string(), // unknown
        ];
        let (mapped, unmapped) = map_refs_to_canonical_ids(&refs, &identities);
        assert_eq!(mapped, vec![ObjectIdentityId(4)]);
        assert_eq!(unmapped, vec!["99 0 R".to_string()]);
    }

    #[test]
    fn map_refs_resolves_source_identity_ids_and_deduplicates() {
        let identities = make_identities();
        let refs = vec![
            "object-4-0-revision-before".to_string(),
            "stream-4-0-revision-before".to_string(),
            "4 0 R".to_string(),
            "object-99-0-revision-before".to_string(),
            "object-99-0-revision-before".to_string(),
        ];
        let (mapped, unmapped) = map_refs_to_canonical_ids(&refs, &identities);
        assert_eq!(mapped, vec![ObjectIdentityId(4)]);
        assert_eq!(unmapped, vec!["object-99-0-revision-before".to_string()]);
    }

    #[test]
    fn nested_write_set_collector_accepts_structured_object_refs() {
        let refs = collect_render_write_set_refs(&json!({
            "report": {
                "render_invalidation": {
                    "changed_font_refs": [[4, 0], {"object_number": 5, "generation": 0}],
                    "changed_image_refs": [
                        {"ref": "stream-3-0-revision-before"},
                        {"appearance_object": 4, "appearance_generation": 0}
                    ],
                    "not_a_write_set": [{"object_number": 99, "generation": 0}]
                }
            }
        }));

        assert_eq!(
            refs,
            vec![
                "3 0 R".to_string(),
                "4 0 R".to_string(),
                "5 0 R".to_string()
            ]
        );
    }

    #[test]
    fn nested_write_set_collector_accepts_camel_case_binding_refs() {
        let refs = collect_render_write_set_refs(&json!({
            "report": {
                "renderInvalidation": {
                    "changedFontRefs": [
                        {"fontObject": 4, "fontGeneration": 0}
                    ],
                    "changedOutputIntentRefs": [
                        {"outputIntentObject": 5, "outputIntentGeneration": 0}
                    ],
                    "renderWriteSetRefs": [
                        {"objectRef": "6 0 R"},
                        {"sourceRef": "stream-7-0-revision-before"}
                    ],
                    "notAWriteSet": [
                        {"fontObject": 99, "fontGeneration": 0}
                    ]
                }
            }
        }));

        assert_eq!(
            refs,
            vec![
                "4 0 R".to_string(),
                "5 0 R".to_string(),
                "6 0 R".to_string(),
                "7 0 R".to_string()
            ]
        );
    }

    #[test]
    fn nested_write_set_collector_accepts_shared_resource_aliases() {
        let refs = collect_render_write_set_refs(&json!({
            "report": {
                "render_invalidation": {
                    "changed_xobject_refs": ["6 0 R"],
                    "changed_color_space_refs": [
                        {"color_space_object": 7, "color_space_generation": 0}
                    ],
                    "changed_ext_gstate_refs": [
                        {"ext_gstate_object": 8, "ext_gstate_generation": 0}
                    ],
                    "shared_resource_refs": [
                        {"resource_dictionary_object": 9, "resource_dictionary_generation": 0}
                    ],
                    "transitive_write_set_refs": [
                        {"ocg_object": 10, "ocg_generation": 0},
                        {"appearance_stream_object": 11, "appearance_stream_generation": 0}
                    ],
                    "changed_optional_content_state_refs": [
                        {"optional_content_state_object": 12, "optional_content_state_generation": 0}
                    ],
                    "changed_form_field_refs": [
                        {"form_field_object": 13, "form_field_generation": 0}
                    ],
                    "changed_form_value_refs": [
                        {"form_value_object": 14, "form_value_generation": 0}
                    ],
                    "changed_type3_glyph_refs": [
                        {"charproc_object": 15, "charproc_generation": 0}
                    ],
                    "changed_font_descriptor_refs": [
                        {"font_descriptor_object": 16, "font_descriptor_generation": 0}
                    ],
                    "changed_cmap_refs": [
                        {"cmap_object": 17, "cmap_generation": 0}
                    ],
                    "changed_image_mask_refs": [
                        {"image_mask_object": 18, "image_mask_generation": 0}
                    ],
                    "changed_soft_mask_refs": [
                        {"soft_mask_object": 19, "soft_mask_generation": 0}
                    ],
                    "not_a_write_set": [
                        {"color_space_object": 99, "color_space_generation": 0}
                    ]
                }
            }
        }));

        assert_eq!(
            refs,
            vec![
                "10 0 R".to_string(),
                "11 0 R".to_string(),
                "12 0 R".to_string(),
                "13 0 R".to_string(),
                "14 0 R".to_string(),
                "15 0 R".to_string(),
                "16 0 R".to_string(),
                "17 0 R".to_string(),
                "18 0 R".to_string(),
                "19 0 R".to_string(),
                "6 0 R".to_string(),
                "7 0 R".to_string(),
                "8 0 R".to_string(),
                "9 0 R".to_string()
            ]
        );
    }

    #[test]
    fn nested_write_set_collector_accepts_category_specific_mutation_aliases() {
        let refs = collect_render_write_set_refs(&json!({
            "report": {
                "renderInvalidation": {
                    "createdImageRefs": [
                        {"createdImageObject": 80, "createdImageGeneration": 0}
                    ],
                    "removedFormXObjectRefs": [
                        {"removedFormXObjectObject": 81, "removedFormXObjectGeneration": 0}
                    ],
                    "deletedOptionalContentConfigRefs": [
                        {"deletedOptionalContentConfigObject": 82, "deletedOptionalContentConfigGeneration": 0}
                    ],
                    "addedAnnotationAppearanceRefs": [
                        {"addedAnnotationAppearanceObject": 83, "addedAnnotationAppearanceGeneration": 0}
                    ],
                    "updatedProofProfileRefs": [
                        {"updatedProofProfileObject": 84, "updatedProofProfileGeneration": 0}
                    ],
                    "modifiedJpxParamsRefs": [
                        {"modifiedJpxParamsObject": 85, "modifiedJpxParamsGeneration": 0}
                    ],
                    "replacedResourceDictionaryRefs": [
                        {"replacedResourceDictionaryObject": 86, "replacedResourceDictionaryGeneration": 0}
                    ],
                    "notAWriteSet": [
                        {"createdImageObject": 99, "createdImageGeneration": 0}
                    ]
                }
            }
        }));

        assert_eq!(
            refs,
            vec![
                "80 0 R".to_string(),
                "81 0 R".to_string(),
                "82 0 R".to_string(),
                "83 0 R".to_string(),
                "84 0 R".to_string(),
                "85 0 R".to_string(),
                "86 0 R".to_string()
            ]
        );
    }

    #[test]
    fn nested_write_set_collector_accepts_annotation_widget_ap_state_aliases() {
        let refs = collect_render_write_set_refs(&json!({
            "report": {
                "renderInvalidation": {
                    "changedAnnotationAPRefs": [
                        {"normalAppearanceObject": 90, "normalAppearanceGeneration": 0}
                    ],
                    "updatedWidgetAPStreamRefs": [
                        {"rolloverApObject": 91, "rolloverApGeneration": 0}
                    ],
                    "removedAPRefs": [
                        {"downAppearanceStreamObject": 92, "downAppearanceStreamGeneration": 0}
                    ],
                    "changedAppearanceStateRefs": [
                        {"appearanceStateObject": 93, "appearanceStateGeneration": 0}
                    ],
                    "beforeAppearanceRefs": ["94 0 R"],
                    "afterWidgetAppearanceRefs": [
                        {"widgetApObject": 95, "widgetApGeneration": 0}
                    ],
                    "notAWriteSet": [
                        {"normalAppearanceObject": 99, "normalAppearanceGeneration": 0}
                    ]
                }
            }
        }));

        assert_eq!(
            refs,
            vec![
                "90 0 R".to_string(),
                "91 0 R".to_string(),
                "92 0 R".to_string(),
                "93 0 R".to_string(),
                "94 0 R".to_string(),
                "95 0 R".to_string()
            ]
        );
    }

    #[test]
    fn broad_write_set_collector_ignores_non_ref_metadata_strings() {
        let refs = collect_render_write_set_refs(&json!({
            "report": {
                "renderInvalidation": {
                    "writeSet": [
                        {
                            "operation": "replace_text",
                            "reason": "local glyph edit",
                            "target": "object-80-0-revision-before",
                            "changedFontRefs": ["not-a-ref"]
                        },
                        {
                            "kind": "annotation_move",
                            "targetObject": 81,
                            "targetGeneration": 0,
                            "notes": ["safe to ignore", "not a pdf ref"]
                        }
                    ],
                    "affectedObjects": [
                        "82 0 R",
                        "semantic-region-title",
                        {"objectNumber": 83, "generation": 0}
                    ]
                }
            }
        }));

        assert_eq!(
            refs,
            vec![
                "80 0 R".to_string(),
                "81 0 R".to_string(),
                "82 0 R".to_string(),
                "83 0 R".to_string(),
                "not-a-ref".to_string()
            ]
        );
    }

    #[test]
    fn nested_write_set_collector_accepts_image_decode_parameter_aliases() {
        let refs = collect_render_write_set_refs(&json!({
            "report": {
                "renderInvalidation": {
                    "changedImageFilterRefs": [
                        {"imageFilterObject": 60, "imageFilterGeneration": 0}
                    ],
                    "changedDecodeArrayRefs": [
                        {"decodeArrayObject": 61, "decodeArrayGeneration": 0}
                    ],
                    "changedDecodeParamsRefs": [
                        {"decodeParamsObject": 62, "decodeParamsGeneration": 0}
                    ],
                    "changedDecodeParmsRefs": [
                        {"decodeparms_object": 63, "decodeparms_generation": 0}
                    ],
                    "changedSourceRegionRefs": [
                        {"sourceRegionObject": 72, "sourceRegionGeneration": 0}
                    ],
                    "changedImageReductionRefs": [
                        {"imageReductionObject": 73, "imageReductionGeneration": 0}
                    ],
                    "changedComponentSelectionRefs": [
                        {"componentSelectionObject": 74, "componentSelectionGeneration": 0}
                    ],
                    "changedCodecTileRefs": [
                        {"codecTileObject": 75, "codecTileGeneration": 0}
                    ],
                    "changedJpxTileRefs": [
                        {"jpxTileObject": 76, "jpxTileGeneration": 0}
                    ],
                    "changedJpxComponentRefs": [
                        {"jpxComponentObject": 77, "jpxComponentGeneration": 0}
                    ],
                    "changedProgressiveImageRefs": [
                        {"progressiveImageObject": 78, "progressiveImageGeneration": 0}
                    ],
                    "changedInterpolationRefs": [
                        {"interpolationObject": 64, "interpolationGeneration": 0}
                    ],
                    "changedColorKeyMaskRefs": [
                        {"colorKeyMaskObject": 65, "colorKeyMaskGeneration": 0}
                    ],
                    "changedImageMatteRefs": [
                        {"image_matte_object": 66, "image_matte_generation": 0}
                    ],
                    "changedSmaskMatteRefs": [
                        {"smask_matte_object": 67, "smask_matte_generation": 0}
                    ],
                    "changedJpegParamsRefs": [
                        {"jpegParamsObject": 68, "jpegParamsGeneration": 0}
                    ],
                    "changedJpxParamsRefs": [
                        {"jpx_params_object": 69, "jpx_params_generation": 0}
                    ],
                    "changedCcittParamsRefs": [
                        {"ccittParamsObject": 70, "ccittParamsGeneration": 0}
                    ],
                    "changedJbig2GlobalsRefs": [
                        {"jbig2GlobalsObject": 71, "jbig2GlobalsGeneration": 0}
                    ],
                    "notAWriteSet": [
                        {"decodeArrayObject": 99, "decodeArrayGeneration": 0}
                    ]
                }
            }
        }));

        assert_eq!(
            refs,
            vec![
                "60 0 R".to_string(),
                "61 0 R".to_string(),
                "62 0 R".to_string(),
                "63 0 R".to_string(),
                "64 0 R".to_string(),
                "65 0 R".to_string(),
                "66 0 R".to_string(),
                "67 0 R".to_string(),
                "68 0 R".to_string(),
                "69 0 R".to_string(),
                "70 0 R".to_string(),
                "71 0 R".to_string(),
                "72 0 R".to_string(),
                "73 0 R".to_string(),
                "74 0 R".to_string(),
                "75 0 R".to_string(),
                "76 0 R".to_string(),
                "77 0 R".to_string(),
                "78 0 R".to_string()
            ]
        );
    }

    #[test]
    fn nested_write_set_collector_accepts_render_structure_aliases() {
        let refs = collect_render_write_set_refs(&json!({
            "report": {
                "render_invalidation": {
                    "changed_content_stream_refs": [
                        {"content_stream_object": 20, "content_stream_generation": 0}
                    ],
                    "changed_page_content_refs": [
                        {"page_content_object": 21, "page_content_generation": 0}
                    ],
                    "changed_page_program_refs": [
                        {"page_program_object": 22, "page_program_generation": 0}
                    ],
                    "changedRetainedOperationRefs": [
                        {"retainedOperationObject": 32, "retainedOperationGeneration": 0}
                    ],
                    "changed_retained_ops_refs": [
                        {"retained_ops_object": 33, "retained_ops_generation": 0}
                    ],
                    "changedDisplayOperationRefs": [
                        {"displayOperationObject": 34, "displayOperationGeneration": 0}
                    ],
                    "changed_display_op_refs": [
                        {"display_op_object": 35, "display_op_generation": 0}
                    ],
                    "changedDisplayListRefs": [
                        {"displayListObject": 36, "displayListGeneration": 0}
                    ],
                    "changed_render_resource_refs": [
                        {"render_resource_object": 37, "render_resource_generation": 0}
                    ],
                    "changedRenderSublistRefs": [
                        {"renderSublistObject": 38, "renderSublistGeneration": 0}
                    ],
                    "changed_form_sublist_refs": [
                        {"form_sublist_object": 39, "form_sublist_generation": 0}
                    ],
                    "changedType3SublistRefs": [
                        {"type3SublistObject": 40, "type3SublistGeneration": 0}
                    ],
                    "changed_pattern_sublist_refs": [
                        {"pattern_sublist_object": 41, "pattern_sublist_generation": 0}
                    ],
                    "changedAppearanceSublistRefs": [
                        {"appearanceSublistObject": 42, "appearanceSublistGeneration": 0}
                    ],
                    "changed_spatial_entry_refs": [
                        {"spatial_entry_object": 43, "spatial_entry_generation": 0}
                    ],
                    "changedSpatialIndexRefs": [
                        {"spatialIndexObject": 44, "spatialIndexGeneration": 0}
                    ],
                    "createdBackendPlanRefs": [
                        {"backendPlanObject": 45, "backendPlanGeneration": 0}
                    ],
                    "changed_backend_plan_cache_refs": [
                        {"backend_plan_cache_object": 46, "backend_plan_cache_generation": 0}
                    ],
                    "changedRenderCacheRefs": [
                        {"renderCacheObject": 47, "renderCacheGeneration": 0}
                    ],
                    "changed_cache_entry_refs": [
                        {"cache_entry_object": 48, "cache_entry_generation": 0}
                    ],
                    "changedTileRefs": [
                        {"tileObject": 49, "tileGeneration": 0}
                    ],
                    "changed_render_tile_refs": [
                        {"render_tile_object": 50, "render_tile_generation": 0}
                    ],
                    "changed_page_resource_dictionary_refs": [
                        {"page_resource_dictionary_object": 23, "page_resource_dictionary_generation": 0}
                    ],
                    "changed_render_relevant_structure_refs": [
                        {"render_relevant_structure_object": 24, "render_relevant_structure_generation": 0}
                    ],
                    "changed_output_intent_refs": [
                        {"output_intent_object": 25, "output_intent_generation": 0}
                    ],
                    "changed_icc_profile_refs": [
                        {"icc_profile_object": 26, "icc_profile_generation": 0}
                    ],
                    "changed_transfer_function_refs": [
                        {"transfer_function_object": 27, "transfer_function_generation": 0}
                    ],
                    "changed_halftone_refs": [
                        {"halftone_object": 28, "halftone_generation": 0}
                    ],
                    "changed_print_profile_refs": [
                        {"print_profile_object": 29, "print_profile_generation": 0}
                    ],
                    "changed_render_contract_refs": [
                        {"render_contract_object": 30, "render_contract_generation": 0}
                    ],
                    "changed_render_structure_refs": [
                        {"render_structure_object": 31, "render_structure_generation": 0}
                    ],
                    "not_a_write_set": [
                        {"render_structure_object": 99, "render_structure_generation": 0}
                    ]
                }
            }
        }));

        assert_eq!(
            refs,
            vec![
                "20 0 R".to_string(),
                "21 0 R".to_string(),
                "22 0 R".to_string(),
                "23 0 R".to_string(),
                "24 0 R".to_string(),
                "25 0 R".to_string(),
                "26 0 R".to_string(),
                "27 0 R".to_string(),
                "28 0 R".to_string(),
                "29 0 R".to_string(),
                "30 0 R".to_string(),
                "31 0 R".to_string(),
                "32 0 R".to_string(),
                "33 0 R".to_string(),
                "34 0 R".to_string(),
                "35 0 R".to_string(),
                "36 0 R".to_string(),
                "37 0 R".to_string(),
                "38 0 R".to_string(),
                "39 0 R".to_string(),
                "40 0 R".to_string(),
                "41 0 R".to_string(),
                "42 0 R".to_string(),
                "43 0 R".to_string(),
                "44 0 R".to_string(),
                "45 0 R".to_string(),
                "46 0 R".to_string(),
                "47 0 R".to_string(),
                "48 0 R".to_string(),
                "49 0 R".to_string(),
                "50 0 R".to_string()
            ]
        );
    }

    #[test]
    fn nested_write_set_collector_accepts_display_proof_profile_aliases() {
        let refs = collect_render_write_set_refs(&json!({
            "report": {
                "renderInvalidation": {
                    "changedDisplayOutputIntentRefs": [
                        {"displayOutputIntentObject": 43, "displayOutputIntentGeneration": 0}
                    ],
                    "changedPrintOutputIntentRefs": [
                        {"print_output_intent_object": 44, "print_output_intent_generation": 0}
                    ],
                    "changedProofOutputIntentRefs": [
                        {"proof_output_intent_object": 45, "proof_output_intent_generation": 0}
                    ],
                    "changedDisplayProfileRefs": [
                        {"displayProfileObject": 46, "displayProfileGeneration": 0}
                    ],
                    "changedProofProfileRefs": [
                        {"proofProfileObject": 47, "proofProfileGeneration": 0}
                    ],
                    "changedProofingProfileRefs": [
                        {"proofing_profile_object": 48, "proofing_profile_generation": 0}
                    ],
                    "changedColorManagementProfileRefs": [
                        {"colorManagementProfileObject": 49, "colorManagementProfileGeneration": 0}
                    ],
                    "changedColorManagementPolicyRefs": [
                        {"color_management_policy_object": 50, "color_management_policy_generation": 0}
                    ],
                    "changedCmmProfileRefs": [
                        {"cmmProfileObject": 51, "cmmProfileGeneration": 0}
                    ],
                    "changedSeparationProfileRefs": [
                        {"separationProfileObject": 52, "separationProfileGeneration": 0}
                    ],
                    "changedDeviceNProfileRefs": [
                        {"deviceNProfileObject": 53, "deviceNProfileGeneration": 0}
                    ],
                    "notAWriteSet": [
                        {"displayProfileObject": 99, "displayProfileGeneration": 0}
                    ]
                }
            }
        }));

        assert_eq!(
            refs,
            vec![
                "43 0 R".to_string(),
                "44 0 R".to_string(),
                "45 0 R".to_string(),
                "46 0 R".to_string(),
                "47 0 R".to_string(),
                "48 0 R".to_string(),
                "49 0 R".to_string(),
                "50 0 R".to_string(),
                "51 0 R".to_string(),
                "52 0 R".to_string(),
                "53 0 R".to_string()
            ]
        );
    }

    #[test]
    fn nested_write_set_collector_accepts_prepress_overprint_aliases() {
        let refs = collect_render_write_set_refs(&json!({
            "report": {
                "renderInvalidation": {
                    "changedRenderingIntentRefs": [
                        {"renderingIntentObject": 54, "renderingIntentGeneration": 0}
                    ],
                    "changedOverprintRefs": [
                        {"overprintObject": 55, "overprintGeneration": 0}
                    ],
                    "changedOverprintStateRefs": [
                        {"overprint_state_object": 56, "overprint_state_generation": 0}
                    ],
                    "changedOverprintModeRefs": [
                        {"overprintModeObject": 57, "overprintModeGeneration": 0}
                    ],
                    "changedPrepressRefs": [
                        {"prepressObject": 58, "prepressGeneration": 0}
                    ],
                    "changedPrepressPolicyRefs": [
                        {"prepress_policy_object": 59, "prepress_policy_generation": 0}
                    ],
                    "changedPrepressPlateRefs": [
                        {"prepressPlateObject": 60, "prepressPlateGeneration": 0}
                    ],
                    "changedPlateRefs": [
                        {"plateObject": 61, "plateGeneration": 0}
                    ],
                    "changedInkRefs": [
                        {"inkObject": 62, "inkGeneration": 0}
                    ],
                    "changedSpotColorRefs": [
                        {"spotColorObject": 63, "spotColorGeneration": 0}
                    ],
                    "changedSpotPlateRefs": [
                        {"spotPlateObject": 64, "spotPlateGeneration": 0}
                    ],
                    "changedSeparationPlateRefs": [
                        {"separation_plate_object": 65, "separation_plate_generation": 0}
                    ],
                    "changedDeviceNPlateRefs": [
                        {"deviceNPlateObject": 66, "deviceNPlateGeneration": 0},
                        {"device_n_plate_object": 67, "device_n_plate_generation": 0}
                    ],
                    "changedBlackPointCompensationRefs": [
                        {"blackPointCompensationObject": 68, "blackPointCompensationGeneration": 0}
                    ],
                    "changedBlackGenerationRefs": [
                        {"blackGenerationObject": 69, "blackGeneration": 0}
                    ],
                    "changedUndercolorRemovalRefs": [
                        {"undercolorRemovalObject": 70, "undercolorRemovalGeneration": 0}
                    ],
                    "changedTrappingRefs": [
                        {"trappingObject": 71, "trappingGeneration": 0}
                    ],
                    "changedTrapNetworkRefs": [
                        {"trapNetworkObject": 72, "trapNetworkGeneration": 0}
                    ],
                    "notAWriteSet": [
                        {"overprintObject": 99, "overprintGeneration": 0}
                    ]
                }
            }
        }));

        assert_eq!(
            refs,
            vec![
                "54 0 R".to_string(),
                "55 0 R".to_string(),
                "56 0 R".to_string(),
                "57 0 R".to_string(),
                "58 0 R".to_string(),
                "59 0 R".to_string(),
                "60 0 R".to_string(),
                "61 0 R".to_string(),
                "62 0 R".to_string(),
                "63 0 R".to_string(),
                "64 0 R".to_string(),
                "65 0 R".to_string(),
                "66 0 R".to_string(),
                "67 0 R".to_string(),
                "68 0 R".to_string(),
                "69 0 R".to_string(),
                "70 0 R".to_string(),
                "71 0 R".to_string(),
                "72 0 R".to_string()
            ]
        );
    }

    #[test]
    fn nested_write_set_collector_accepts_document_structure_and_group_aliases() {
        let refs = collect_render_write_set_refs(&json!({
            "report": {
                "renderInvalidation": {
                    "changedPageTreeRefs": [
                        {"pageTreeObject": 32, "pageTreeGeneration": 0}
                    ],
                    "changedCatalogRefs": [
                        {"catalogObject": 33, "catalogGeneration": 0}
                    ],
                    "changedMetadataRefs": [
                        {"xmpMetadataObject": 34, "xmpMetadataGeneration": 0}
                    ],
                    "changedNameTreeRefs": [
                        {"nameTreeObject": 35, "nameTreeGeneration": 0}
                    ],
                    "changedStructTreeRefs": [
                        {"structTreeObject": 36, "structTreeGeneration": 0}
                    ],
                    "changedParentTreeRefs": [
                        {"parentTreeObject": 37, "parentTreeGeneration": 0}
                    ],
                    "changedRoleMapRefs": [
                        {"roleMapObject": 38, "roleMapGeneration": 0}
                    ],
                    "changedTransparencyGroupRefs": [
                        {"transparencyGroupObject": 39, "transparencyGroupGeneration": 0}
                    ],
                    "changedImageSmaskRefs": [
                        {"imageSmaskObject": 40, "imageSmaskGeneration": 0}
                    ],
                    "changedWidgetAppearanceRefs": [
                        {"widgetAppearanceObject": 41, "widgetAppearanceGeneration": 0}
                    ],
                    "changedOptionalContentConfigRefs": [
                        {"optionalContentConfigObject": 42, "optionalContentConfigGeneration": 0}
                    ],
                    "notAWriteSet": [
                        {"catalogObject": 99, "catalogGeneration": 0}
                    ]
                }
            }
        }));

        assert_eq!(
            refs,
            vec![
                "32 0 R".to_string(),
                "33 0 R".to_string(),
                "34 0 R".to_string(),
                "35 0 R".to_string(),
                "36 0 R".to_string(),
                "37 0 R".to_string(),
                "38 0 R".to_string(),
                "39 0 R".to_string(),
                "40 0 R".to_string(),
                "41 0 R".to_string(),
                "42 0 R".to_string()
            ]
        );
    }

    #[test]
    fn nested_write_set_collector_accepts_optional_content_properties_aliases() {
        let refs = collect_render_write_set_refs(&json!({
            "report": {
                "renderInvalidation": {
                    "changedOCPropertiesRefs": [
                        {"ocPropertiesObject": 4, "ocPropertiesGeneration": 0}
                    ],
                    "changedPropertiesRefs": [
                        {"propertiesObject": 5, "propertiesGeneration": 0}
                    ],
                    "changedOptionalContentRefs": [
                        {"optionalContentObject": 6, "optionalContentGeneration": 0}
                    ],
                    "notAWriteSet": [
                        {"propertiesObject": 99, "propertiesGeneration": 0}
                    ]
                }
            }
        }));

        assert_eq!(
            refs,
            vec![
                "4 0 R".to_string(),
                "5 0 R".to_string(),
                "6 0 R".to_string()
            ]
        );
    }

    #[test]
    fn dirty_regions_convert_to_intersecting_render_tiles() {
        let viewport = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let tiles = dirty_regions_to_render_tiles(
            &[json!({
                "page": 2,
                "region": [16.0, 84.0, 31.0, 99.0],
                "reason": "unit_test"
            })],
            2,
            &viewport,
            16,
            16,
        );

        assert_eq!(
            tiles,
            vec![(
                2,
                RenderTile {
                    x: 16,
                    y: 0,
                    width: 16,
                    height: 16,
                }
            )]
        );
    }

    #[test]
    fn dirty_regions_accept_camel_case_page_and_bounds_aliases() {
        let viewport = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let tiles = dirty_regions_to_render_tiles(
            &[json!({
                "pageNumber": 2,
                "dirtyRegion": [16.0, 84.0, 31.0, 99.0],
                "reason": "unit_test"
            })],
            2,
            &viewport,
            16,
            16,
        );

        assert_eq!(
            tiles,
            vec![(
                2,
                RenderTile {
                    x: 16,
                    y: 0,
                    width: 16,
                    height: 16,
                }
            )]
        );
    }

    #[test]
    fn dirty_regions_accept_annotation_widget_and_appearance_bounds_aliases() {
        let viewport = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let tiles = dirty_regions_to_render_tiles(
            &[
                json!({
                    "pageNumber": 2,
                    "annotationBounds": [16.0, 84.0, 31.0, 99.0],
                    "reason": "annotation_rect_before",
                }),
                json!({
                    "pageNumber": 2,
                    "widgetRect": [32.0, 68.0, 47.0, 83.0],
                    "reason": "widget_rect_after",
                }),
                json!({
                    "pageNumber": 2,
                    "appearanceBounds": [48.0, 52.0, 63.0, 67.0],
                    "reason": "appearance_bounds_after",
                }),
            ],
            2,
            &viewport,
            16,
            16,
        );

        assert_eq!(
            tiles,
            vec![
                (
                    2,
                    RenderTile {
                        x: 16,
                        y: 0,
                        width: 16,
                        height: 16,
                    },
                ),
                (
                    2,
                    RenderTile {
                        x: 32,
                        y: 16,
                        width: 16,
                        height: 16,
                    },
                ),
                (
                    2,
                    RenderTile {
                        x: 48,
                        y: 32,
                        width: 16,
                        height: 16,
                    },
                )
            ]
        );
    }

    #[test]
    fn dirty_regions_skip_invalid_or_other_page_entries() {
        let viewport = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let tiles = dirty_regions_to_render_tiles(
            &[
                json!({"page": 1, "region": [0.0, 0.0, 100.0, 100.0]}),
                json!({"page": 2, "region": [0.0, 0.0, 0.0, 100.0]}),
                json!({"page": 2, "region": ["bad", 0.0, 10.0, 10.0]}),
            ],
            2,
            &viewport,
            16,
            16,
        );

        assert!(tiles.is_empty());
    }

    #[test]
    fn exact_tile_mode_selector_allows_source_only_transactions() {
        let tile = RenderTile {
            x: 0,
            y: 0,
            width: 64,
            height: 64,
        };

        assert!(exact_tiles_cover_all_reported_pages(&[], &[]));
        assert!(exact_tiles_cover_all_reported_pages(&[], &[(2, tile)]));
        assert!(exact_tiles_cover_all_reported_pages(&[2], &[(2, tile)]));
        assert!(!exact_tiles_cover_all_reported_pages(&[2], &[]));
        assert!(!exact_tiles_cover_all_reported_pages(&[2], &[(3, tile)]));
    }

    #[test]
    fn transaction_write_set_json_accepts_camel_case_fields_and_tile_objects() {
        let tile = RenderTile {
            x: 64,
            y: 0,
            width: 64,
            height: 64,
        };
        let write_set_json = json!({
            "affectedObjectRefs": ["4 0 R"],
            "affectedPages": [1],
            "affectedTiles": [{"pageNumber": 1, "tile": tile}],
            "nextRevision": RevisionId(2),
        })
        .to_string();

        let write_set: TransactionWriteSet = serde_json::from_str(&write_set_json).unwrap();

        assert_eq!(write_set.affected_object_refs, vec!["4 0 R".to_string()]);
        assert_eq!(write_set.affected_pages, vec![1]);
        assert_eq!(write_set.affected_tiles, vec![(1, tile)]);
        assert_eq!(write_set.next_revision, RevisionId(2));
    }

    #[test]
    fn transaction_write_set_json_preserves_tuple_tile_shape() {
        let tile = RenderTile {
            x: 64,
            y: 0,
            width: 64,
            height: 64,
        };
        let write_set_json = json!({
            "affected_object_refs": ["4 0 R"],
            "affected_pages": [1],
            "affected_tiles": [[1, tile]],
            "next_revision": RevisionId(2),
        })
        .to_string();

        let write_set: TransactionWriteSet = serde_json::from_str(&write_set_json).unwrap();

        assert_eq!(write_set.affected_tiles, vec![(1, tile)]);
    }

    #[test]
    fn local_edit_invalidates_page_1_retains_page_2_cache() {
        let mut cache = RenderDocumentCache::new();
        cache.bind_document_revision(RevisionId(1));

        // Record dependencies: object 4 -> page 1, object 5 -> page 2
        cache.record_page_source_dependency(1, ObjectIdentityId(4));
        cache.record_page_source_dependency(2, ObjectIdentityId(5));
        cache.record_tile_dependency(
            1,
            RenderTile {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            },
        );
        cache.record_tile_dependency(
            2,
            RenderTile {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            },
        );

        let identities = make_identities();

        // Transaction affects only object 4 (content stream for page 1)
        let write_set = TransactionWriteSet::from_transaction_report(
            &["4 0 R".to_string()],
            &[1],
            RevisionId(2),
        );

        let result = write_set.invalidate(&mut cache, &identities);

        // Page 1 should be invalidated
        assert!(result.invalidation.invalidated_pages.contains(&1));
        // Page 2 should NOT be invalidated
        assert!(!result.invalidation.invalidated_pages.contains(&2));
        // No unmapped refs
        assert!(result.unmapped_refs.is_empty());
        // Not a full reset
        assert!(!result.invalidation.cache_must_reset);
    }

    #[test]
    fn sequential_source_only_edits_retain_dependency_edges() {
        let mut cache = RenderDocumentCache::new();
        cache.bind_document_revision(RevisionId(1));

        let page_1_tile = RenderTile {
            x: 0,
            y: 0,
            width: 64,
            height: 64,
        };
        let page_2_tile = RenderTile {
            x: 64,
            y: 0,
            width: 64,
            height: 64,
        };
        cache.record_page_source_dependency(1, ObjectIdentityId(4));
        cache.record_page_source_dependency(2, ObjectIdentityId(5));
        cache.record_tile_dependency(1, page_1_tile);
        cache.record_tile_dependency(2, page_2_tile);

        let identities = make_identities();
        let first = TransactionWriteSet::from_transaction_report(
            &["4 0 R".to_string()],
            &[],
            RevisionId(2),
        )
        .invalidate(&mut cache, &identities);

        assert_eq!(first.invalidation.invalidated_pages, vec![1]);
        assert_eq!(first.invalidation.invalidated_tiles, vec![(1, page_1_tile)]);
        assert!(!first.invalidation.cache_must_reset);
        assert_eq!(cache.document_revision(), Some(RevisionId(2)));

        let second = TransactionWriteSet::from_transaction_report(
            &["5 0 R".to_string()],
            &[],
            RevisionId(3),
        )
        .invalidate(&mut cache, &identities);

        assert_eq!(second.invalidation.invalidated_pages, vec![2]);
        assert_eq!(
            second.invalidation.invalidated_tiles,
            vec![(2, page_2_tile)]
        );
        assert!(!second.invalidation.cache_must_reset);
        assert_eq!(cache.document_revision(), Some(RevisionId(3)));
    }

    #[test]
    fn transaction_invalidation_registers_source_cache_markers() {
        let mut cache = RenderDocumentCache::new();
        cache.bind_document_revision(RevisionId(1));
        cache.record_tile_source_dependency(
            1,
            ObjectIdentityId(4),
            RenderTile {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            },
        );

        let identities = make_identities();
        let write_set = TransactionWriteSet::from_transaction_report(
            &["4 0 R".to_string()],
            &[],
            RevisionId(2),
        );
        let result = write_set.invalidate(&mut cache, &identities);
        let markers = cache.source_cache_markers_for_changed_sources(&[ObjectIdentityId(4)]);

        assert!(result.unmapped_refs.is_empty());
        assert!(markers.contains(&"xobject:4:0:".to_string()));
        assert!(markers.contains(&"annotation-appearance:ref:4:0:".to_string()));
        assert!(markers.contains(&"tiling-program:ref:4:0:".to_string()));
        assert!(markers.contains(&"smask-transfer:ref:4:0:".to_string()));
    }

    #[test]
    fn unknown_object_triggers_conservative_reset() {
        let mut cache = RenderDocumentCache::new();
        cache.bind_document_revision(RevisionId(1));
        cache.record_page_source_dependency(1, ObjectIdentityId(4));
        cache.record_tile_dependency(
            1,
            RenderTile {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            },
        );

        let identities = make_identities();

        // Transaction reports an object ref that doesn't exist in our identity table
        let write_set = TransactionWriteSet::from_transaction_report(
            &["99 0 R".to_string()],
            &[1],
            RevisionId(2),
        );

        let result = write_set.invalidate(&mut cache, &identities);

        // Must do conservative reset because we can't map the ref
        assert!(result.invalidation.cache_must_reset);
        assert_eq!(result.unmapped_refs, vec!["99 0 R".to_string()]);
    }

    #[test]
    fn mixed_known_and_unknown_refs_trigger_conservative_reset() {
        let mut cache = RenderDocumentCache::new();
        cache.bind_document_revision(RevisionId(1));
        cache.record_page_source_dependency(1, ObjectIdentityId(4));

        let identities = make_identities();

        let write_set = TransactionWriteSet::from_transaction_report(
            &["4 0 R".to_string(), "unknown_ref".to_string()],
            &[1],
            RevisionId(2),
        );

        let result = write_set.invalidate(&mut cache, &identities);

        // Even though object 4 is known, the unknown ref forces conservative reset
        assert!(result.invalidation.cache_must_reset);
        assert_eq!(result.unmapped_refs, vec!["unknown_ref".to_string()]);
    }

    #[test]
    fn page_only_transaction_invalidates_reported_page_without_full_reset() {
        let mut cache = RenderDocumentCache::new();
        cache.bind_document_revision(RevisionId(1));
        cache.record_tile_dependency(
            2,
            RenderTile {
                x: 4,
                y: 8,
                width: 16,
                height: 32,
            },
        );

        let write_set = TransactionWriteSet::from_transaction_report(&[], &[2], RevisionId(2));
        let result = write_set.invalidate(&mut cache, &make_identities());

        assert_eq!(result.invalidation.invalidated_pages, vec![2]);
        assert_eq!(result.invalidation.invalidated_tiles.len(), 1);
        assert!(!result.invalidation.cache_must_reset);
        assert!(result.mapped_ids.is_empty());
        assert!(result.unmapped_refs.is_empty());
    }

    #[test]
    fn explicit_tile_transaction_invalidates_tile_without_page_artifacts() {
        let mut cache = RenderDocumentCache::new();
        cache.bind_document_revision(RevisionId(1));
        let tile = RenderTile {
            x: 128,
            y: 0,
            width: 64,
            height: 64,
        };

        let write_set = TransactionWriteSet::from_transaction_report_with_tiles(
            &[],
            &[],
            &[(2, tile)],
            RevisionId(2),
        );
        let result = write_set.invalidate(&mut cache, &make_identities());

        assert!(result.invalidation.invalidated_pages.is_empty());
        assert_eq!(result.invalidation.invalidated_tiles, vec![(2, tile)]);
        assert!(!result.invalidation.cache_must_reset);
        assert!(result.mapped_ids.is_empty());
        assert!(result.unmapped_refs.is_empty());
    }

    #[test]
    fn explicit_tiles_with_reported_page_do_not_expand_all_page_tiles() {
        let mut cache = RenderDocumentCache::new();
        cache.bind_document_revision(RevisionId(1));
        let clean_tile = RenderTile {
            x: 0,
            y: 0,
            width: 64,
            height: 64,
        };
        let dirty_tile = RenderTile {
            x: 64,
            y: 0,
            width: 64,
            height: 64,
        };
        cache.record_tile_dependency(2, clean_tile);
        cache.record_tile_dependency(2, dirty_tile);

        let write_set = TransactionWriteSet::from_transaction_report_with_tiles(
            &[],
            &[2],
            &[(2, dirty_tile)],
            RevisionId(2),
        );
        let result = write_set.invalidate(&mut cache, &make_identities());

        assert_eq!(result.invalidation.invalidated_pages, vec![2]);
        assert_eq!(result.invalidation.invalidated_tiles, vec![(2, dirty_tile)]);
        assert!(!result.invalidation.cache_must_reset);
    }

    #[test]
    fn zero_sized_explicit_tile_does_not_mask_page_wide_invalidation() {
        let mut cache = RenderDocumentCache::new();
        cache.bind_document_revision(RevisionId(1));
        let clean_tile = RenderTile {
            x: 0,
            y: 0,
            width: 64,
            height: 64,
        };
        let dirty_tile = RenderTile {
            x: 64,
            y: 0,
            width: 64,
            height: 64,
        };
        let zero_tile = RenderTile {
            x: 64,
            y: 0,
            width: 0,
            height: 64,
        };
        cache.record_tile_dependency(2, clean_tile);
        cache.record_tile_dependency(2, dirty_tile);

        let write_set = TransactionWriteSet::from_transaction_report_with_tiles(
            &[],
            &[2],
            &[(2, zero_tile)],
            RevisionId(2),
        );
        let result = write_set.invalidate(&mut cache, &make_identities());

        assert_eq!(result.invalidation.invalidated_pages, vec![2]);
        assert_eq!(
            result.invalidation.invalidated_tiles,
            vec![(2, clean_tile), (2, dirty_tile)]
        );
        assert!(!result.invalidation.cache_must_reset);
    }

    #[test]
    fn explicit_tiles_expand_reported_pages_without_exact_tile_coverage() {
        let mut cache = RenderDocumentCache::new();
        cache.bind_document_revision(RevisionId(1));
        let page_2_clean = RenderTile {
            x: 0,
            y: 0,
            width: 64,
            height: 64,
        };
        let page_2_dirty = RenderTile {
            x: 64,
            y: 0,
            width: 64,
            height: 64,
        };
        let page_3_tile = RenderTile {
            x: 0,
            y: 64,
            width: 64,
            height: 64,
        };
        cache.record_tile_dependency(2, page_2_clean);
        cache.record_tile_dependency(2, page_2_dirty);
        cache.record_tile_dependency(3, page_3_tile);

        let write_set = TransactionWriteSet::from_transaction_report_with_tiles(
            &[],
            &[2, 3],
            &[(2, page_2_dirty)],
            RevisionId(2),
        );
        let result = write_set.invalidate(&mut cache, &make_identities());

        assert_eq!(result.invalidation.invalidated_pages, vec![2, 3]);
        assert_eq!(
            result.invalidation.invalidated_tiles,
            vec![(2, page_2_clean), (2, page_2_dirty), (3, page_3_tile)]
        );
        assert!(!result.invalidation.cache_must_reset);
    }

    #[test]
    fn exact_tiles_expand_source_pages_without_exact_tile_coverage() {
        let mut cache = RenderDocumentCache::new();
        cache.bind_document_revision(RevisionId(1));
        let page_1_a = RenderTile {
            x: 0,
            y: 0,
            width: 64,
            height: 64,
        };
        let page_1_b = RenderTile {
            x: 64,
            y: 0,
            width: 64,
            height: 64,
        };
        let page_2_dirty = RenderTile {
            x: 0,
            y: 64,
            width: 64,
            height: 64,
        };
        cache.record_page_source_dependency(1, ObjectIdentityId(4));
        cache.record_tile_dependency(1, page_1_a);
        cache.record_tile_dependency(1, page_1_b);

        let write_set = TransactionWriteSet::from_transaction_report_with_tiles(
            &["4 0 R".to_string()],
            &[],
            &[(2, page_2_dirty)],
            RevisionId(2),
        );
        let result = write_set.invalidate(&mut cache, &make_identities());

        assert_eq!(result.invalidation.invalidated_pages, vec![1]);
        assert_eq!(
            result.invalidation.invalidated_tiles,
            vec![(1, page_1_a), (1, page_1_b), (2, page_2_dirty)]
        );
        assert!(!result.invalidation.cache_must_reset);
    }

    #[test]
    fn exact_tiles_expand_coarse_source_pages_with_partial_source_tile_coverage() {
        let mut cache = RenderDocumentCache::new();
        cache.bind_document_revision(RevisionId(1));
        let page_1_source_tile = RenderTile {
            x: 0,
            y: 0,
            width: 64,
            height: 64,
        };
        let page_1_recorded_tile = RenderTile {
            x: 64,
            y: 0,
            width: 64,
            height: 64,
        };
        let page_2_dirty = RenderTile {
            x: 0,
            y: 64,
            width: 64,
            height: 64,
        };
        cache.record_page_source_dependency(1, ObjectIdentityId(4));
        cache.record_tile_source_dependency(1, ObjectIdentityId(4), page_1_source_tile);
        cache.record_tile_dependency(1, page_1_recorded_tile);

        let write_set = TransactionWriteSet::from_transaction_report_with_tiles(
            &["4 0 R".to_string()],
            &[],
            &[(2, page_2_dirty)],
            RevisionId(2),
        );
        let result = write_set.invalidate(&mut cache, &make_identities());

        assert_eq!(result.invalidation.invalidated_pages, vec![1]);
        assert_eq!(
            result.invalidation.invalidated_tiles,
            vec![
                (1, page_1_source_tile),
                (1, page_1_recorded_tile),
                (2, page_2_dirty),
            ]
        );
        assert!(!result.invalidation.cache_must_reset);
    }

    #[test]
    fn render_invalidation_plan_json_applies_exact_tiles_to_cache() {
        let mut cache = RenderDocumentCache::new();
        cache.bind_document_revision(RevisionId(1));
        let tile = RenderTile {
            x: 128,
            y: 64,
            width: 64,
            height: 64,
        };
        let plan_json = json!({
            "schema_version": RENDER_TRANSACTION_INVALIDATION_PLAN_SCHEMA_VERSION,
            "next_revision": RevisionId(2),
            "mapped_source_ids": [],
            "affected_pages": [],
            "affected_tiles": [{"page": 3, "tile": tile}],
            "conservative_reset_required": false,
        })
        .to_string();

        let result = apply_render_invalidation_plan_json_to_cache(&mut cache, &plan_json).unwrap();

        assert_eq!(result.previous_revision, RevisionId(1));
        assert_eq!(result.current_revision, RevisionId(2));
        assert!(result.invalidated_pages.is_empty());
        assert_eq!(result.invalidated_tiles, vec![(3, tile)]);
        assert!(!result.cache_must_reset);
    }

    #[test]
    fn render_invalidation_plan_json_with_page_uses_exact_tiles() {
        let mut cache = RenderDocumentCache::new();
        cache.bind_document_revision(RevisionId(1));
        let clean_tile = RenderTile {
            x: 0,
            y: 0,
            width: 64,
            height: 64,
        };
        let dirty_tile = RenderTile {
            x: 64,
            y: 0,
            width: 64,
            height: 64,
        };
        cache.record_tile_dependency(3, clean_tile);
        cache.record_tile_dependency(3, dirty_tile);
        let plan_json = json!({
            "schema_version": RENDER_TRANSACTION_INVALIDATION_PLAN_SCHEMA_VERSION,
            "next_revision": RevisionId(2),
            "mapped_source_ids": [],
            "affected_pages": [3],
            "affected_tiles": [{"page": 3, "tile": dirty_tile}],
            "conservative_reset_required": false,
        })
        .to_string();

        let result = apply_render_invalidation_plan_json_to_cache(&mut cache, &plan_json).unwrap();

        assert_eq!(result.invalidated_pages, vec![3]);
        assert_eq!(result.invalidated_tiles, vec![(3, dirty_tile)]);
        assert!(!result.cache_must_reset);
    }

    #[test]
    fn render_invalidation_plan_merges_dirty_region_tiles_before_apply() {
        let mut cache = RenderDocumentCache::new();
        cache.bind_document_revision(RevisionId(1));
        let clean_tile = RenderTile {
            x: 0,
            y: 0,
            width: 16,
            height: 16,
        };
        let dirty_tile = RenderTile {
            x: 16,
            y: 0,
            width: 16,
            height: 16,
        };
        cache.record_tile_dependency(3, clean_tile);
        cache.record_tile_dependency(3, dirty_tile);
        let plan_json = json!({
            "schema_version": RENDER_TRANSACTION_INVALIDATION_PLAN_SCHEMA_VERSION,
            "next_revision": RevisionId(2),
            "mapped_source_ids": [],
            "affected_pages": [3],
            "affected_tiles": [],
            "conservative_reset_required": false,
        })
        .to_string();
        let mut plan = RenderInvalidationCachePlan::from_json(&plan_json).unwrap();
        let viewport = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);

        let added = plan.merge_dirty_region_tiles(
            &[json!({
                "page": 3,
                "region": [16.0, 84.0, 31.0, 99.0]
            })],
            3,
            &viewport,
            16,
            16,
        );
        let duplicate = plan.merge_dirty_region_tiles(
            &[json!({
                "page": 3,
                "region": [16.0, 84.0, 31.0, 99.0]
            })],
            3,
            &viewport,
            16,
            16,
        );

        assert_eq!(added, 1);
        assert_eq!(duplicate, 0);
        let result = plan.apply_to_cache(&mut cache);

        assert_eq!(result.invalidated_pages, vec![3]);
        assert_eq!(result.invalidated_tiles, vec![(3, dirty_tile)]);
        assert!(!result.cache_must_reset);
    }

    #[test]
    fn render_invalidation_plan_json_zero_sized_tile_does_not_cover_page() {
        let mut cache = RenderDocumentCache::new();
        cache.bind_document_revision(RevisionId(1));
        let clean_tile = RenderTile {
            x: 0,
            y: 0,
            width: 64,
            height: 64,
        };
        let dirty_tile = RenderTile {
            x: 64,
            y: 0,
            width: 64,
            height: 64,
        };
        let zero_tile = RenderTile {
            x: 64,
            y: 0,
            width: 0,
            height: 64,
        };
        cache.record_tile_dependency(3, clean_tile);
        cache.record_tile_dependency(3, dirty_tile);
        let plan_json = json!({
            "schema_version": RENDER_TRANSACTION_INVALIDATION_PLAN_SCHEMA_VERSION,
            "next_revision": RevisionId(2),
            "mapped_source_ids": [],
            "affected_pages": [3],
            "affected_tiles": [{"page": 3, "tile": zero_tile}],
            "conservative_reset_required": false,
        })
        .to_string();

        let result = apply_render_invalidation_plan_json_to_cache(&mut cache, &plan_json).unwrap();

        assert_eq!(result.invalidated_pages, vec![3]);
        assert_eq!(
            result.invalidated_tiles,
            vec![(3, clean_tile), (3, dirty_tile)]
        );
        assert!(!result.cache_must_reset);
    }

    #[test]
    fn render_invalidation_plan_json_expands_uncovered_reported_pages() {
        let mut cache = RenderDocumentCache::new();
        cache.bind_document_revision(RevisionId(1));
        let page_3_clean = RenderTile {
            x: 0,
            y: 0,
            width: 64,
            height: 64,
        };
        let page_3_dirty = RenderTile {
            x: 64,
            y: 0,
            width: 64,
            height: 64,
        };
        let page_4_tile = RenderTile {
            x: 0,
            y: 64,
            width: 64,
            height: 64,
        };
        cache.record_tile_dependency(3, page_3_clean);
        cache.record_tile_dependency(3, page_3_dirty);
        cache.record_tile_dependency(4, page_4_tile);
        let plan_json = json!({
            "schema_version": RENDER_TRANSACTION_INVALIDATION_PLAN_SCHEMA_VERSION,
            "next_revision": RevisionId(2),
            "mapped_source_ids": [],
            "affected_pages": [3, 4],
            "affected_tiles": [{"page": 3, "tile": page_3_dirty}],
            "conservative_reset_required": false,
        })
        .to_string();

        let result = apply_render_invalidation_plan_json_to_cache(&mut cache, &plan_json).unwrap();

        assert_eq!(result.invalidated_pages, vec![3, 4]);
        assert_eq!(
            result.invalidated_tiles,
            vec![(3, page_3_clean), (3, page_3_dirty), (4, page_4_tile)]
        );
        assert!(!result.cache_must_reset);
    }

    #[test]
    fn render_invalidation_plan_json_registers_source_cache_markers() {
        let mut cache = RenderDocumentCache::new();
        cache.bind_document_revision(RevisionId(1));
        let tile = RenderTile {
            x: 0,
            y: 0,
            width: 64,
            height: 64,
        };
        cache.record_tile_source_dependency(1, ObjectIdentityId(4), tile);
        let plan_json = json!({
            "schema_version": RENDER_TRANSACTION_INVALIDATION_PLAN_SCHEMA_VERSION,
            "next_revision": RevisionId(2),
            "mapped_source_ids": [ObjectIdentityId(4)],
            "source_cache_markers": [{
                "source_id": ObjectIdentityId(4),
                "object_number": 4,
                "generation": 0,
                "markers": ["annotation-appearance:ref:4:0:"],
            }],
            "affected_pages": [],
            "affected_tiles": [],
            "conservative_reset_required": false,
        })
        .to_string();

        let result = apply_render_invalidation_plan_json_to_cache(&mut cache, &plan_json).unwrap();
        let markers = cache.source_cache_markers_for_changed_sources(&[ObjectIdentityId(4)]);

        assert_eq!(result.previous_revision, RevisionId(1));
        assert_eq!(result.current_revision, RevisionId(2));
        assert_eq!(result.invalidated_tiles, vec![(1, tile)]);
        assert!(!result.cache_must_reset);
        assert!(markers.contains(&"annotation-appearance:ref:4:0:".to_string()));
    }

    #[test]
    fn render_invalidation_plan_json_accepts_camel_case_plan_fields() {
        let mut cache = RenderDocumentCache::new();
        cache.bind_document_revision(RevisionId(1));
        let tile = RenderTile {
            x: 0,
            y: 0,
            width: 64,
            height: 64,
        };
        cache.record_tile_source_dependency(1, ObjectIdentityId(4), tile);
        let plan_json = json!({
            "schemaVersion": RENDER_TRANSACTION_INVALIDATION_PLAN_SCHEMA_VERSION,
            "nextRevision": RevisionId(2),
            "mappedSourceIds": [ObjectIdentityId(4)],
            "sourceCacheMarkers": [{
                "sourceId": ObjectIdentityId(4),
                "objectNumber": 4,
                "generationNumber": 0,
                "markers": ["annotation-appearance:ref:4:0:"],
            }],
            "affectedPages": [],
            "affectedTiles": [],
            "conservativeResetRequired": false,
        })
        .to_string();

        let result = apply_render_invalidation_plan_json_to_cache(&mut cache, &plan_json).unwrap();
        let markers = cache.source_cache_markers_for_changed_sources(&[ObjectIdentityId(4)]);

        assert_eq!(result.previous_revision, RevisionId(1));
        assert_eq!(result.current_revision, RevisionId(2));
        assert_eq!(result.invalidated_tiles, vec![(1, tile)]);
        assert!(!result.cache_must_reset);
        assert!(markers.contains(&"annotation-appearance:ref:4:0:".to_string()));
    }

    #[test]
    fn render_invalidation_plan_json_accepts_camel_case_envelope_and_tile_fields() {
        let mut cache = RenderDocumentCache::new();
        cache.bind_document_revision(RevisionId(1));
        let clean_tile = RenderTile {
            x: 0,
            y: 0,
            width: 64,
            height: 64,
        };
        let dirty_tile = RenderTile {
            x: 64,
            y: 0,
            width: 64,
            height: 64,
        };
        cache.record_tile_dependency(3, clean_tile);
        cache.record_tile_dependency(3, dirty_tile);
        let envelope_json = json!({
            "schemaVersion": 1,
            "kind": "editing_transactions_apply_with_render_invalidation",
            "report": {
                "renderInvalidation": {
                    "schemaVersion": RENDER_TRANSACTION_INVALIDATION_PLAN_SCHEMA_VERSION,
                    "nextRevision": RevisionId(2),
                    "mappedSourceIds": [],
                    "sourceCacheMarkers": [],
                    "affectedPages": [3],
                    "affectedTiles": [{"pageNumber": 3, "tile": dirty_tile}],
                    "conservativeResetRequired": false,
                }
            }
        })
        .to_string();

        let result =
            apply_render_invalidation_plan_json_to_cache(&mut cache, &envelope_json).unwrap();

        assert_eq!(result.previous_revision, RevisionId(1));
        assert_eq!(result.current_revision, RevisionId(2));
        assert_eq!(result.invalidated_pages, vec![3]);
        assert_eq!(result.invalidated_tiles, vec![(3, dirty_tile)]);
        assert!(!result.cache_must_reset);
    }

    #[test]
    fn render_invalidation_plan_json_derives_empty_source_cache_markers() {
        let mut cache = RenderDocumentCache::new();
        cache.bind_document_revision(RevisionId(1));
        let tile = RenderTile {
            x: 0,
            y: 0,
            width: 64,
            height: 64,
        };
        cache.record_tile_source_dependency(1, ObjectIdentityId(4), tile);
        let plan_json = json!({
            "schema_version": RENDER_TRANSACTION_INVALIDATION_PLAN_SCHEMA_VERSION,
            "next_revision": RevisionId(2),
            "mapped_source_ids": [ObjectIdentityId(4)],
            "source_cache_markers": [{
                "source_id": ObjectIdentityId(4),
                "object_number": 4,
                "generation": 0,
            }],
            "affected_pages": [],
            "affected_tiles": [],
            "conservative_reset_required": false,
        })
        .to_string();

        let result = apply_render_invalidation_plan_json_to_cache(&mut cache, &plan_json).unwrap();
        let markers = cache.source_cache_markers_for_changed_sources(&[ObjectIdentityId(4)]);

        assert_eq!(result.invalidated_tiles, vec![(1, tile)]);
        assert!(!result.cache_must_reset);
        assert!(markers.contains(&"annotation-appearance:ref:4:0:".to_string()));
        assert!(markers.contains(&"tiling-program:ref:4:0:".to_string()));
    }

    #[test]
    fn render_invalidation_plan_json_maps_nested_render_write_set_refs() {
        let mut cache = RenderDocumentCache::new();
        cache.bind_document_revision(RevisionId(1));
        cache.remember_source_identities(&make_identities());
        let tile = RenderTile {
            x: 0,
            y: 0,
            width: 64,
            height: 64,
        };
        cache.record_tile_source_dependency(1, ObjectIdentityId(4), tile);
        let envelope_json = json!({
            "schema_version": 1,
            "kind": "document_subsystems_apply_with_render_invalidation",
            "report": {
                "render_invalidation": {
                    "schema_version": RENDER_TRANSACTION_INVALIDATION_PLAN_SCHEMA_VERSION,
                    "next_revision": RevisionId(2),
                    "mapped_source_ids": [],
                    "source_cache_markers": [],
                    "affected_pages": [],
                    "affected_tiles": [],
                    "conservative_reset_required": false,
                    "render_write_set_refs": ["4 0 R"],
                    "changed_object_refs": ["stream-4-0-revision-before"],
                }
            }
        })
        .to_string();

        let result =
            apply_render_invalidation_plan_json_to_cache(&mut cache, &envelope_json).unwrap();
        let markers = cache.source_cache_markers_for_changed_sources(&[ObjectIdentityId(4)]);

        assert_eq!(result.invalidated_tiles, vec![(1, tile)]);
        assert!(!result.cache_must_reset);
        assert!(markers.contains(&"xobject:4:0:".to_string()));
        assert!(markers.contains(&"annotation-appearance:ref:4:0:".to_string()));
    }

    #[test]
    fn render_invalidation_plan_json_maps_annotation_widget_ap_alias_refs() {
        let mut cache = RenderDocumentCache::new();
        cache.bind_document_revision(RevisionId(1));
        cache.remember_source_identities(&make_identities());
        let annotation_tile = RenderTile {
            x: 0,
            y: 0,
            width: 64,
            height: 64,
        };
        let widget_tile = RenderTile {
            x: 64,
            y: 0,
            width: 64,
            height: 64,
        };
        cache.record_tile_source_dependency(1, ObjectIdentityId(4), annotation_tile);
        cache.record_tile_source_dependency(1, ObjectIdentityId(5), widget_tile);
        let envelope_json = json!({
            "schema_version": 1,
            "kind": "document_subsystems_apply_with_render_invalidation",
            "report": {
                "renderInvalidation": {
                    "schemaVersion": RENDER_TRANSACTION_INVALIDATION_PLAN_SCHEMA_VERSION,
                    "nextRevision": RevisionId(2),
                    "mappedSourceIds": [],
                    "sourceCacheMarkers": [],
                    "affectedPages": [],
                    "affectedTiles": [],
                    "conservativeResetRequired": false,
                    "changedAnnotationAPRefs": [
                        {"normalAppearanceObject": 4, "normalAppearanceGeneration": 0}
                    ],
                    "afterWidgetAppearanceRefs": [
                        {"widgetApObject": 5, "widgetApGeneration": 0}
                    ],
                }
            }
        })
        .to_string();

        let result =
            apply_render_invalidation_plan_json_to_cache(&mut cache, &envelope_json).unwrap();
        let annotation_markers =
            cache.source_cache_markers_for_changed_sources(&[ObjectIdentityId(4)]);
        let widget_markers = cache.source_cache_markers_for_changed_sources(&[ObjectIdentityId(5)]);

        assert_eq!(
            result.invalidated_tiles,
            vec![(1, annotation_tile), (1, widget_tile)]
        );
        assert!(!result.cache_must_reset);
        assert!(annotation_markers.contains(&"annotation-appearance:ref:4:0:".to_string()));
        assert!(widget_markers.contains(&"annotation-appearance:ref:5:0:".to_string()));
    }

    #[test]
    fn render_invalidation_plan_json_unknown_ap_alias_ref_resets_cache() {
        let mut cache = RenderDocumentCache::new();
        cache.bind_document_revision(RevisionId(1));
        cache.remember_source_identities(&make_identities());
        cache.record_page_source_dependency(1, ObjectIdentityId(4));
        let envelope_json = json!({
            "schema_version": 1,
            "kind": "document_subsystems_apply_with_render_invalidation",
            "report": {
                "renderInvalidation": {
                    "schemaVersion": RENDER_TRANSACTION_INVALIDATION_PLAN_SCHEMA_VERSION,
                    "nextRevision": RevisionId(2),
                    "mappedSourceIds": [],
                    "sourceCacheMarkers": [],
                    "affectedPages": [],
                    "affectedTiles": [],
                    "conservativeResetRequired": false,
                    "changedAPRefs": [
                        {"apObject": 99, "apGeneration": 0}
                    ],
                }
            }
        })
        .to_string();

        let result =
            apply_render_invalidation_plan_json_to_cache(&mut cache, &envelope_json).unwrap();

        assert!(result.cache_must_reset);
        assert!(result.invalidated_pages.is_empty());
        assert!(result.invalidated_tiles.is_empty());
        assert_eq!(cache.document_revision(), Some(RevisionId(2)));
    }

    #[test]
    fn render_invalidation_plan_json_ignores_broad_write_set_metadata_strings() {
        let mut cache = RenderDocumentCache::new();
        cache.bind_document_revision(RevisionId(1));
        cache.remember_source_identities(&make_identities());
        let tile = RenderTile {
            x: 0,
            y: 0,
            width: 64,
            height: 64,
        };
        cache.record_tile_source_dependency(1, ObjectIdentityId(4), tile);
        let envelope_json = json!({
            "schema_version": 1,
            "kind": "editing_transactions_apply_with_render_invalidation",
            "report": {
                "render_invalidation": {
                    "schema_version": RENDER_TRANSACTION_INVALIDATION_PLAN_SCHEMA_VERSION,
                    "next_revision": RevisionId(2),
                    "mapped_source_ids": [],
                    "source_cache_markers": [],
                    "affected_pages": [],
                    "affected_tiles": [],
                    "conservative_reset_required": false,
                    "writeSet": [{
                        "operation": "replace_text",
                        "reason": "local glyph edit",
                        "target": "object-4-0-revision-before"
                    }],
                    "affectedObjects": [
                        "semantic-region-title",
                        {"objectNumber": 4, "generation": 0}
                    ]
                }
            }
        })
        .to_string();

        let result =
            apply_render_invalidation_plan_json_to_cache(&mut cache, &envelope_json).unwrap();

        assert_eq!(result.invalidated_tiles, vec![(1, tile)]);
        assert!(!result.cache_must_reset);
        assert_eq!(cache.document_revision(), Some(RevisionId(2)));
    }

    #[test]
    fn render_invalidation_plan_json_maps_structured_shared_resource_refs() {
        let mut cache = RenderDocumentCache::new();
        cache.bind_document_revision(RevisionId(1));
        cache.remember_source_identities(&make_identities());
        let font_tile = RenderTile {
            x: 0,
            y: 0,
            width: 64,
            height: 64,
        };
        let image_tile = RenderTile {
            x: 64,
            y: 0,
            width: 64,
            height: 64,
        };
        cache.record_tile_source_dependency(1, ObjectIdentityId(4), font_tile);
        cache.record_tile_source_dependency(2, ObjectIdentityId(5), image_tile);
        let envelope_json = json!({
            "schema_version": 1,
            "kind": "editing_transactions_apply_with_render_invalidation",
            "report": {
                "render_invalidation": {
                    "schema_version": RENDER_TRANSACTION_INVALIDATION_PLAN_SCHEMA_VERSION,
                    "next_revision": RevisionId(2),
                    "mapped_source_ids": [],
                    "source_cache_markers": [],
                    "affected_pages": [],
                    "affected_tiles": [],
                    "conservative_reset_required": false,
                    "changed_font_refs": [[4, 0]],
                    "changed_xobject_refs": [{"xobject_object": 4, "xobject_generation": 0}],
                    "changed_color_space_refs": [
                        {"color_space_object": 4, "color_space_generation": 0}
                    ],
                    "changed_image_refs": [{"object_number": 5, "generation": 0}],
                    "changed_ext_gstate_refs": [
                        {"ext_gstate_object": 5, "ext_gstate_generation": 0}
                    ],
                    "shared_resource_refs": [
                        {"resource_dictionary_object": 5, "resource_dictionary_generation": 0}
                    ],
                    "transitive_write_set_refs": ["object-4-0-revision-before"],
                }
            }
        })
        .to_string();

        let result =
            apply_render_invalidation_plan_json_to_cache(&mut cache, &envelope_json).unwrap();
        let font_markers = cache.source_cache_markers_for_changed_sources(&[ObjectIdentityId(4)]);
        let image_markers = cache.source_cache_markers_for_changed_sources(&[ObjectIdentityId(5)]);

        assert_eq!(
            result.invalidated_tiles,
            vec![(1, font_tile), (2, image_tile)]
        );
        assert!(!result.cache_must_reset);
        assert!(font_markers.contains(&"xobject:4:0:".to_string()));
        assert!(image_markers.contains(&"xobject:5:0:".to_string()));
    }

    #[test]
    fn render_invalidation_plan_json_maps_image_decode_tile_component_refs() {
        let mut cache = RenderDocumentCache::new();
        cache.bind_document_revision(RevisionId(1));
        cache.remember_source_identities(&make_identities());
        let component_tile = RenderTile {
            x: 0,
            y: 64,
            width: 64,
            height: 64,
        };
        let jpx_tile = RenderTile {
            x: 64,
            y: 64,
            width: 64,
            height: 64,
        };
        cache.record_tile_source_dependency(1, ObjectIdentityId(4), component_tile);
        cache.record_tile_source_dependency(2, ObjectIdentityId(5), jpx_tile);
        let envelope_json = json!({
            "schema_version": 1,
            "kind": "editing_transactions_apply_with_render_invalidation",
            "report": {
                "renderInvalidation": {
                    "schemaVersion": RENDER_TRANSACTION_INVALIDATION_PLAN_SCHEMA_VERSION,
                    "nextRevision": RevisionId(2),
                    "mappedSourceIds": [],
                    "sourceCacheMarkers": [],
                    "affectedPages": [],
                    "affectedTiles": [],
                    "conservativeResetRequired": false,
                    "changedComponentSelectionRefs": [
                        {"componentSelectionObject": 4, "componentSelectionGeneration": 0}
                    ],
                    "changedJpxTileRefs": [
                        {"jpxTileObject": 5, "jpxTileGeneration": 0}
                    ],
                }
            }
        })
        .to_string();

        let result =
            apply_render_invalidation_plan_json_to_cache(&mut cache, &envelope_json).unwrap();
        let component_markers =
            cache.source_cache_markers_for_changed_sources(&[ObjectIdentityId(4)]);
        let jpx_markers = cache.source_cache_markers_for_changed_sources(&[ObjectIdentityId(5)]);

        assert_eq!(
            result.invalidated_tiles,
            vec![(1, component_tile), (2, jpx_tile)]
        );
        assert!(!result.cache_must_reset);
        assert!(component_markers.contains(&"image-decode:ref:4:0:".to_string()));
        assert!(jpx_markers.contains(&"image-decode:ref:5:0:".to_string()));
    }

    #[test]
    fn render_invalidation_plan_json_maps_optional_content_properties_refs() {
        let mut cache = RenderDocumentCache::new();
        cache.bind_document_revision(RevisionId(1));
        cache.remember_source_identities(&make_identities());
        let oc_properties_tile = RenderTile {
            x: 0,
            y: 0,
            width: 64,
            height: 64,
        };
        let properties_tile = RenderTile {
            x: 64,
            y: 0,
            width: 64,
            height: 64,
        };
        cache.record_tile_source_dependency(1, ObjectIdentityId(4), oc_properties_tile);
        cache.record_tile_source_dependency(1, ObjectIdentityId(5), properties_tile);
        let envelope_json = json!({
            "schema_version": 1,
            "kind": "editing_transactions_apply_with_render_invalidation",
            "report": {
                "renderInvalidation": {
                    "schemaVersion": RENDER_TRANSACTION_INVALIDATION_PLAN_SCHEMA_VERSION,
                    "nextRevision": RevisionId(2),
                    "mappedSourceIds": [],
                    "sourceCacheMarkers": [],
                    "affectedPages": [],
                    "affectedTiles": [],
                    "conservativeResetRequired": false,
                    "changedOCPropertiesRefs": [
                        {"ocPropertiesObject": 4, "ocPropertiesGeneration": 0}
                    ],
                    "changedPropertiesRefs": [
                        {"propertiesObject": 5, "propertiesGeneration": 0}
                    ],
                }
            }
        })
        .to_string();

        let result =
            apply_render_invalidation_plan_json_to_cache(&mut cache, &envelope_json).unwrap();
        let oc_markers = cache.source_cache_markers_for_changed_sources(&[ObjectIdentityId(4)]);
        let properties_markers =
            cache.source_cache_markers_for_changed_sources(&[ObjectIdentityId(5)]);

        assert_eq!(
            result.invalidated_tiles,
            vec![(1, oc_properties_tile), (1, properties_tile)]
        );
        assert!(!result.cache_must_reset);
        assert!(oc_markers.contains(&"properties:ref:4:0:".to_string()));
        assert!(properties_markers.contains(&"properties:ref:5:0:".to_string()));
    }

    #[test]
    fn render_invalidation_plan_json_unknown_nested_ref_resets_cache() {
        let mut cache = RenderDocumentCache::new();
        cache.bind_document_revision(RevisionId(1));
        cache.remember_source_identities(&make_identities());
        cache.record_page_source_dependency(1, ObjectIdentityId(4));
        let envelope_json = json!({
            "schema_version": 1,
            "kind": "secure_mutation_incremental_form_render_invalidation",
            "report": {
                "render_invalidation": {
                    "schema_version": RENDER_TRANSACTION_INVALIDATION_PLAN_SCHEMA_VERSION,
                    "next_revision": RevisionId(2),
                    "mapped_source_ids": [],
                    "source_cache_markers": [],
                    "affected_pages": [],
                    "affected_tiles": [],
                    "conservative_reset_required": false,
                    "created_object_refs": ["99 0 R"]
                }
            }
        })
        .to_string();

        let result =
            apply_render_invalidation_plan_json_to_cache(&mut cache, &envelope_json).unwrap();

        assert!(result.cache_must_reset);
        assert!(result.invalidated_pages.is_empty());
        assert!(result.invalidated_tiles.is_empty());
        assert_eq!(cache.document_revision(), Some(RevisionId(2)));
    }

    #[test]
    fn render_invalidation_plan_envelope_triggers_conservative_cache_reset() {
        let mut cache = RenderDocumentCache::new();
        cache.bind_document_revision(RevisionId(1));
        cache.record_page_source_dependency(1, ObjectIdentityId(4));
        let envelope_json = json!({
            "schema_version": 1,
            "kind": "editing_transactions_transaction_apply_with_render_invalidation",
            "report": {
                "render_invalidation": {
                    "schema_version": RENDER_TRANSACTION_INVALIDATION_PLAN_SCHEMA_VERSION,
                    "next_revision": RevisionId(2),
                    "mapped_source_ids": [ObjectIdentityId(4)],
                    "affected_pages": [1],
                    "affected_tiles": [],
                    "conservative_reset_required": true,
                }
            }
        })
        .to_string();

        let result =
            apply_render_invalidation_plan_json_to_cache(&mut cache, &envelope_json).unwrap();

        assert_eq!(result.previous_revision, RevisionId(1));
        assert_eq!(result.current_revision, RevisionId(2));
        assert!(result.cache_must_reset);
        assert!(result.invalidated_pages.is_empty());
        assert_eq!(cache.document_revision(), Some(RevisionId(2)));
    }

    #[test]
    fn render_invalidation_plan_rejects_unknown_schema() {
        let plan_json = json!({
            "schema_version": "render-transaction-invalidation-plan.v0",
            "next_revision": RevisionId(2),
        })
        .to_string();

        assert!(RenderInvalidationCachePlan::from_json(&plan_json).is_err());
    }

    #[test]
    fn mapped_source_and_reported_page_are_unioned() {
        let mut cache = RenderDocumentCache::new();
        cache.bind_document_revision(RevisionId(1));
        cache.record_page_source_dependency(1, ObjectIdentityId(4));
        cache.record_tile_dependency(
            1,
            RenderTile {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            },
        );
        cache.record_tile_dependency(
            2,
            RenderTile {
                x: 64,
                y: 0,
                width: 64,
                height: 64,
            },
        );

        let write_set = TransactionWriteSet::from_transaction_report(
            &["4 0 R".to_string()],
            &[2],
            RevisionId(2),
        );
        let result = write_set.invalidate(&mut cache, &make_identities());

        assert_eq!(result.invalidation.invalidated_pages, vec![1, 2]);
        assert_eq!(result.invalidation.invalidated_tiles.len(), 2);
        assert!(!result.invalidation.cache_must_reset);
    }
}
