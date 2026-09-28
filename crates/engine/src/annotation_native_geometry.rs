//! Native, atomic annotation geometry editing without lossy XFDF reconstruction.
use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotationGeometryChange {
    pub annotation_id: String,
    pub page: usize,
    /// Normalized rectangle in the destination page's default user space.
    pub rect: [f64; 4],
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotationGeometryBatchReport {
    pub input_sha256: String,
    pub output_sha256: String,
    pub edits: Vec<AnnotationGeometryEditReport>,
    pub changed_pages: Vec<usize>,
    #[serde(default)]
    pub promoted_source_ids: Vec<String>,
    /// One normalization receipt per transaction, never cloned per batch row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_normalization: Option<AnnotationPromotionReport>,
    pub exact_limits: Vec<String>,
}

/// Explicitly select every popup/reply member to transform a group. Group
/// members must share the same affine transform and target page. A supplied
/// source hash binds even legacy named IDs to the displayed document revision.
pub fn edit_annotation_geometries_pdf(
    input: &[u8],
    expected_source_sha256: Option<&str>,
    changes: &[AnnotationGeometryChange],
) -> Result<(Vec<u8>, AnnotationGeometryBatchReport)> {
    let input_sha256 = resource_digest(input);
    if expected_source_sha256.is_some_and(|h| !h.eq_ignore_ascii_case(&input_sha256)) {
        return Err(WellfriendError::invalid_input(
            "annotation geometry source revision changed",
        ));
    }
    if changes.is_empty() || changes.len() > 4096 {
        return Err(WellfriendError::invalid_input(
            "annotation geometry requires 1..4096 explicit changes",
        ));
    }
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let identities = crate::annotation_identity::index(engine.document(), MAX_ANNOTATIONS)?;
    let selected = changes
        .iter()
        .map(|c| c.annotation_id.clone())
        .collect::<BTreeSet<_>>();
    let prepared = crate::annotation_promotion::prepare_if_direct(
        input,
        &identities,
        &selected,
        MAX_ANNOTATIONS,
    )?;
    let prepared_input = prepared
        .as_ref()
        .map(|(bytes, _)| bytes.as_slice())
        .unwrap_or(input);
    let engine = ContentEngine::open_bytes(prepared_input.to_vec())?;
    let reader = engine.document().reader();
    let identities = crate::annotation_identity::index(engine.document(), MAX_ANNOTATIONS)?;
    let by_id = identities
        .values()
        .map(|i| (i.id.clone(), i))
        .collect::<BTreeMap<_, _>>();
    let mut moves = Vec::new();
    let mut geometry = BTreeMap::new();
    let mut changed_pages = BTreeSet::new();
    if let Some((_, report)) = &prepared {
        changed_pages.extend(report.changed_pages.iter().copied());
    }
    let mut seen = BTreeSet::new();
    for change in changes {
        crate::cancel::check_current_cancel("native annotation geometry planning")?;
        if change.annotation_id.is_empty() || !seen.insert(change.annotation_id.clone()) {
            return Err(WellfriendError::invalid_input(
                "duplicate or empty annotation geometry identity",
            ));
        }
        let identity = by_id.get(&change.annotation_id).ok_or_else(|| {
            WellfriendError::invalid_input(
                "annotation geometry source identity missing; rediscover the source",
            )
        })?;
        let (number, generation) = identity.reference.ok_or_else(|| {
            WellfriendError::UnsupportedFeature(
                "annotation source was not materialized for native geometry".into(),
            )
        })?;
        let object = reader.get_object(number, generation)?;
        let dict = object
            .as_dict()
            .ok_or_else(|| WellfriendError::invalid_input("invalid annotation object"))?;
        let value = reader.resolve(
            dict.get("Rect")
                .ok_or_else(|| WellfriendError::invalid_input("annotation rectangle missing"))?
                .clone(),
        )?;
        let values = value
            .as_array()
            .ok_or_else(|| WellfriendError::invalid_input("invalid annotation rectangle"))?;
        if values.len() != 4 {
            return Err(WellfriendError::invalid_input(
                "invalid annotation rectangle",
            ));
        }
        let mut old_rect = [0.0; 4];
        for (index, value) in values.iter().enumerate() {
            old_rect[index] = reader.resolve(value.clone())?.as_number().ok_or_else(|| {
                WellfriendError::invalid_input("invalid annotation rectangle coordinate")
            })?;
        }
        if old_rect.iter().any(|v| !v.is_finite()) {
            return Err(WellfriendError::invalid_input(
                "nonfinite annotation source rectangle",
            ));
        }
        old_rect = [
            old_rect[0].min(old_rect[2]),
            old_rect[1].min(old_rect[3]),
            old_rect[0].max(old_rect[2]),
            old_rect[1].max(old_rect[3]),
        ];
        moves.push(crate::story_anchors::StoryAnnotationMove {
            annotation_id: change.annotation_id.clone(),
            source_page: identity.page,
            target_page: change.page,
            old_rect,
            new_rect: change.rect,
            name_change: None,
        });
        let fields = [
            ("Rect", "rect"),
            ("Vertices", "vertices"),
            ("QuadPoints", "quad_points"),
            ("L", "line"),
            ("CL", "callout"),
            ("InkList", "ink_lists"),
            ("RD", "rectangle_differences"),
            ("LL", "leader_length"),
            ("LLE", "leader_extension"),
            ("LLO", "leader_offset"),
            ("CO", "caption_offset"),
        ]
        .iter()
        .filter(|(key, _)| dict.contains_key(key))
        .map(|(_, name)| name.to_string())
        .collect::<Vec<_>>();
        geometry.insert(change.annotation_id.clone(), fields);
        changed_pages.extend([identity.page, change.page]);
    }
    let output = crate::story_anchors::apply_geometry(prepared_input, &moves, true)?;
    let output_sha256 = resource_digest(&output);
    let limits: Vec<String>=vec![
        "native incremental geometry update: original action, appearance, field-owner and opaque dictionary entries are retained; active content is not executed or sanitized".into(),
        "existing appearance programs fit the new Rect; this operation does not reflow FreeText, regenerate missing appearances, or certify rendering fidelity".into(),
        "page rotation and UserUnit must match; related groups need complete selection and a common transform; destination NM conflicts are not silently renamed".into(),
        "direct occurrences and field ancestors normalize through unique field/OBJR ownership; contradictory or ambiguous owners, direct structure owners, Path/Measure/ExData and nonuniform leader/caption resizing remain bounded; signature validity and permission profiles are not certified".into(),
        "incremental history remains; this operation is not sanitizing redaction".into(),
    ];
    let mut edits = Vec::new();
    for movement in moves {
        let canonical_import = AnnotationXfdfImportReport {
            schema_version: ANNOTATION_MEDIA_REDACTION_SCHEMA_VERSION.into(),
            imported_annotations: 0,
            created: 0,
            updated: 1,
            deleted: 0,
            unchanged: 0,
            unsupported: 0,
            duplicate_ids: Vec::new(),
            relationship_count: 0,
            relationship_transaction: Default::default(),
            appearances_regenerated: 0,
            output_bytes: output.len(),
            output_sha256: output_sha256.clone(),
            deterministic: true,
            signature_impact: "incremental_revision_signature_status_not_qualified".into(),
            diagnostics: Vec::new(),
            exact_limits: limits.clone(),
        };
        edits.push(AnnotationGeometryEditReport {
            schema_version: ANNOTATION_MEDIA_REDACTION_SCHEMA_VERSION.into(),
            transformed_geometry: geometry.remove(&movement.annotation_id).unwrap_or_default(),
            annotation_id: movement.annotation_id,
            source_page: movement.source_page,
            output_page: movement.target_page,
            affected_pages: changed_pages.iter().copied().collect(),
            source_normalization: None,
            old_rect: movement.old_rect,
            new_rect: movement.new_rect,
            writer: "native_incremental_geometry_transaction".into(),
            canonical_import,
        });
    }
    let source_normalization = prepared.map(|(_, report)| report);
    let report = AnnotationGeometryBatchReport {
        input_sha256,
        output_sha256,
        edits,
        changed_pages: changed_pages.into_iter().collect(),
        promoted_source_ids: source_normalization
            .as_ref()
            .map(|r| r.promoted_ids.clone())
            .unwrap_or_default(),
        source_normalization,
        exact_limits: limits,
    };
    Ok((output, report))
}
