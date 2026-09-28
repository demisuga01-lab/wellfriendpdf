//! Unexecuted source regressions for shared native/XFDF annotation identities.
use super::*;
use crate::authoring::{PageSize, PdfBuilder};
use crate::writer::{write_incremental_update, IncrementalObject};

fn r(number: u32) -> PdfObject {
    PdfObject::Reference {
        number,
        generation: 0,
    }
}
fn rect(values: [f64; 4]) -> PdfObject {
    PdfObject::Array(values.into_iter().map(PdfObject::Real).collect())
}
fn fixture() -> Vec<u8> {
    let mut builder = PdfBuilder::new();
    builder.add_page(PageSize::custom(200.0, 200.0));
    builder.add_page(PageSize::custom(200.0, 200.0));
    let doc = PdfDocument::open_bytes(builder.to_bytes().unwrap()).unwrap();
    let reader = doc.reader();
    let pages = doc.get_pages().unwrap();
    let n = reader.object_ids().iter().map(|p| p.0).max().unwrap() + 1;
    let mut objects = Vec::new();
    for (i, (page, subtype, name)) in [
        (1, "Text", Some("same name")),
        (1, "Popup", None),
        (2, "Text", Some("same name")),
        (2, "Line", None),
    ]
    .into_iter()
    .enumerate()
    {
        let mut d = PdfDictionary::empty();
        d.insert("Type", PdfObject::Name("Annot".into()));
        d.insert("Subtype", PdfObject::Name(subtype.into()));
        if let Some(name) = name {
            d.insert("NM", pdf_text_string(name));
        }
        d.insert("P", r(pages[page - 1].object_number));
        d.insert(
            "Rect",
            rect([
                10.0 + (i as f64) * 25.0,
                20.0,
                30.0 + (i as f64) * 25.0,
                50.0,
            ]),
        );
        d.insert("Contents", pdf_text_string(&format!("annotation {i} 注釈")));
        if i == 0 {
            d.insert("Popup", r(n + 1));
        }
        if i == 1 {
            d.insert("Parent", r(n));
        }
        if i == 3 {
            d.insert("L", rect([85.0, 20.0, 105.0, 50.0]));
        }
        objects.push(IncrementalObject {
            number: n + i as u32,
            generation: 0,
            object: PdfObject::Dictionary(d),
        });
    }
    for (index, page) in pages.iter().enumerate() {
        let mut d = reader
            .get_object(page.object_number, page.generation_number)
            .unwrap()
            .as_dict()
            .unwrap()
            .clone();
        d.insert(
            "Annots",
            PdfObject::Array(vec![r(n + 2 * index as u32), r(n + 2 * index as u32 + 1)]),
        );
        objects.push(IncrementalObject {
            number: page.object_number,
            generation: page.generation_number,
            object: PdfObject::Dictionary(d),
        });
    }
    write_incremental_update(reader, objects).unwrap()
}
fn change_catalog(input: &[u8]) -> Vec<u8> {
    let doc = PdfDocument::open_bytes(input.to_vec()).unwrap();
    let id = doc.reader().root_reference().unwrap();
    let mut d = doc.get_catalog().unwrap();
    d.insert("WFTestRevision", PdfObject::Integer(1));
    write_incremental_update(
        doc.reader(),
        vec![IncrementalObject {
            number: id.0,
            generation: id.1,
            object: PdfObject::Dictionary(d),
        }],
    )
    .unwrap()
}
fn preserve() -> AnnotationXfdfImportOptions {
    AnnotationXfdfImportOptions {
        appearance_policy: AnnotationAppearancePolicy::PreserveValid,
        ..Default::default()
    }
}
fn external(name: &str, page: usize) -> Vec<u8> {
    format!("<xfdf xmlns='{XFDF_NAMESPACE}'><annots><text name='{}' page='{page}' rect='10,20,30,50'><contents>Updated</contents></text></annots></xfdf>",xml_escape(name)).into_bytes()
}

#[test]
fn shared_ids_roundtrip_duplicate_names_anonymous_objects_and_popup_relationships() {
    let input = fixture();
    let engine = ContentEngine::open_bytes(input.clone()).unwrap();
    let native = crate::story_anchors::annotation_anchor_sources(&input).unwrap();
    let (xfdf, _) = export_annotation_xfdf(&engine).unwrap();
    let parsed = parse_annotation_xfdf(&xfdf).unwrap();
    assert_eq!(
        parsed.source_sha256.as_deref(),
        Some(resource_digest(&input).as_str())
    );
    let ids = parsed
        .annotations
        .iter()
        .map(|a| a.id.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(ids.len(), 4);
    assert_eq!(
        ids,
        native
            .iter()
            .map(|s| s.annotation_id.clone())
            .collect::<BTreeSet<_>>()
    );
    let same = parsed
        .annotations
        .iter()
        .filter(|a| a.pdf_name.as_deref() == Some("same name"))
        .collect::<Vec<_>>();
    assert_eq!(same.len(), 2);
    assert_ne!(same[0].id, same[1].id);
    let popup = parsed
        .annotations
        .iter()
        .find(|a| a.subtype == "Popup")
        .unwrap();
    assert!(ids.contains(popup.popup_for.as_ref().unwrap()));
    let (output, report) = import_annotation_xfdf_pdf(&input, &xfdf, &preserve()).unwrap();
    assert_eq!(report.updated, 4);
    assert_eq!(report.created, 0);
    let e = ContentEngine::open_bytes(output.clone()).unwrap();
    let (next, _) = export_annotation_xfdf(&e).unwrap();
    let next = parse_annotation_xfdf(&next).unwrap();
    assert_eq!(
        ids,
        next.annotations
            .iter()
            .map(|a| a.id.clone())
            .collect::<BTreeSet<_>>()
    );
    for before in &parsed.annotations {
        let after = next.annotations.iter().find(|a| a.id == before.id).unwrap();
        assert_eq!(after.pdf_name, before.pdf_name);
        assert_eq!(after.popup_for, before.popup_for);
        assert_eq!(after.page, before.page);
    }
    let (again, _) = import_annotation_xfdf_pdf(
        &output,
        write_annotation_xfdf(&next).as_bytes(),
        &preserve(),
    )
    .unwrap();
    assert_eq!(
        crate::annotation_identity::index(
            &PdfDocument::open_bytes(again).unwrap(),
            MAX_ANNOTATIONS
        )
        .unwrap()
        .values()
        .map(|a| a.id.clone())
        .collect::<BTreeSet<_>>(),
        ids
    );
}

#[test]
fn stale_export_and_unknown_scoped_ids_cannot_become_new_annotations() {
    let input = fixture();
    let engine = ContentEngine::open_bytes(input.clone()).unwrap();
    let (xfdf, _) = export_annotation_xfdf(&engine).unwrap();
    let changed = change_catalog(&input);
    assert!(import_annotation_xfdf_pdf(&changed, &xfdf, &preserve()).is_err());
    let mut unbound = parse_annotation_xfdf(&xfdf).unwrap();
    unbound.source_sha256 = None;
    assert!(import_annotation_xfdf_pdf(
        &changed,
        write_annotation_xfdf(&unbound).as_bytes(),
        &preserve()
    )
    .is_err());
    assert!(
        import_annotation_xfdf_pdf(&input, &external("wf-anonymous:stale", 0), &preserve())
            .is_err()
    );
}

#[test]
fn extension_attributes_are_bound_to_namespace_uri_not_a_literal_prefix() {
    let input = fixture();
    let engine = ContentEngine::open_bytes(input.clone()).unwrap();
    let (xfdf, _) = export_annotation_xfdf(&engine).unwrap();
    let text = String::from_utf8(xfdf).unwrap();
    let alternate = text
        .replace("xmlns:wellfriendpdf=", "xmlns:sdk=")
        .replace(" wellfriendpdf:", " sdk:");
    let parsed = parse_annotation_xfdf(alternate.as_bytes()).unwrap();
    assert_eq!(
        parsed.source_sha256.as_deref(),
        Some(resource_digest(&input).as_str())
    );
    assert_eq!(
        parsed
            .annotations
            .iter()
            .filter(|a| a.pdf_name.is_some())
            .count(),
        2
    );
    import_annotation_xfdf_pdf(&input, alternate.as_bytes(), &preserve()).unwrap();
    let spoof = text.replace(WELLFRIENDPDF_XFDF_NAMESPACE, "urn:unrelated");
    assert!(parse_annotation_xfdf(spoof.as_bytes())
        .unwrap()
        .source_sha256
        .is_none());
    let duplicate=format!("<xfdf xmlns='{XFDF_NAMESPACE}' xmlns:a='{WELLFRIENDPDF_XFDF_NAMESPACE}' xmlns:b='{WELLFRIENDPDF_XFDF_NAMESPACE}' a:source-sha256='{}' b:source-sha256='{}'><annots/></xfdf>",resource_digest(&input),resource_digest(&input));
    assert!(parse_annotation_xfdf(duplicate.as_bytes()).is_err());
}

#[test]
fn unbound_ambiguous_raw_names_reject_but_unique_nm_aliases_resolve_persisted_ids() {
    let input = fixture();
    assert!(import_annotation_xfdf_pdf(&input, &external("same name", 0), &preserve()).is_err());
    let doc = PdfDocument::open_bytes(input).unwrap();
    let identities = crate::annotation_identity::index(&doc, MAX_ANNOTATIONS).unwrap();
    let first = identities
        .values()
        .find(|i| i.page == 1 && i.name.is_some())
        .unwrap();
    let (number, generation) = first.reference.unwrap();
    let mut d = doc
        .reader()
        .get_object(number, generation)
        .unwrap()
        .as_dict()
        .unwrap()
        .clone();
    d.insert("NM", pdf_text_string("unique display name"));
    d.insert(
        crate::annotation_identity::STABLE_ID,
        pdf_text_string("stable-source"),
    );
    let renamed = write_incremental_update(
        doc.reader(),
        vec![IncrementalObject {
            number,
            generation,
            object: PdfObject::Dictionary(d),
        }],
    )
    .unwrap();
    let (output, report) =
        import_annotation_xfdf_pdf(&renamed, &external("unique display name", 0), &preserve())
            .unwrap();
    assert_eq!(report.updated, 1);
    assert_eq!(report.created, 0);
    let saved = PdfDocument::open_bytes(output).unwrap();
    let index = crate::annotation_identity::index(&saved, MAX_ANNOTATIONS).unwrap();
    let source = index.values().find(|i| i.id == "stable-source").unwrap();
    assert_eq!(source.name.as_deref(), Some("unique display name"));
    let id = source.reference.unwrap();
    let object = saved.reader().get_object(id.0, id.1).unwrap();
    assert_eq!(
        object
            .as_dict()
            .unwrap()
            .get("Contents")
            .and_then(pdf_text_or_name)
            .as_deref(),
        Some("Updated")
    );
}

#[test]
fn appearance_generation_uses_the_same_ids_and_persists_only_generated_owners() {
    let input = fixture();
    let doc = PdfDocument::open_bytes(input.clone()).unwrap();
    let original = crate::annotation_identity::index(&doc, MAX_ANNOTATIONS).unwrap();
    let (output, report) = generate_annotation_appearances_pdf(
        &input,
        &AnnotationAppearanceOptions {
            policy: AnnotationAppearancePolicy::RegenerateAllSupported,
            ..Default::default()
        },
    )
    .unwrap();
    let saved = PdfDocument::open_bytes(output).unwrap();
    let next = crate::annotation_identity::index(&saved, MAX_ANNOTATIONS).unwrap();
    assert!(report.generated > 0);
    for row in report.rows.iter().filter(|r| r.result == "generated") {
        let old = original
            .values()
            .find(|i| i.id == row.annotation_id)
            .unwrap();
        let new = next.values().find(|i| i.id == row.annotation_id).unwrap();
        assert_eq!(old.name, new.name);
        assert_eq!(old.page, new.page);
        assert_eq!(new.provenance, "persisted_editing_id");
    }
}

#[test]
fn direct_annotations_are_not_duplicated_and_cross_page_name_collisions_are_not_silent() {
    let input = fixture();
    let engine = ContentEngine::open_bytes(input.clone()).unwrap();
    let (xfdf, _) = export_annotation_xfdf(&engine).unwrap();
    let mut selected = parse_annotation_xfdf(&xfdf).unwrap();
    selected
        .annotations
        .retain(|a| a.page == 2 && a.pdf_name.is_some());
    selected.annotations[0].page = 1;
    assert!(import_annotation_xfdf_pdf(
        &input,
        write_annotation_xfdf(&selected).as_bytes(),
        &preserve()
    )
    .is_err());
    let doc = PdfDocument::open_bytes(input).unwrap();
    let p = doc.get_page(1).unwrap();
    let reader = doc.reader();
    let mut page = reader
        .get_object(p.object_number, p.generation_number)
        .unwrap()
        .as_dict()
        .unwrap()
        .clone();
    let mut annots = reader
        .resolve(page.get("Annots").unwrap().clone())
        .unwrap()
        .as_array()
        .unwrap()
        .to_vec();
    let direct = reader.resolve(annots[0].clone()).unwrap();
    // Use an independent direct note; no popup/reply edges refer to it.
    let mut direct = direct.as_dict().unwrap().clone();
    direct.remove("Popup");
    direct.insert("NM", pdf_text_string("direct-note"));
    annots.push(PdfObject::Dictionary(direct));
    page.insert("Annots", PdfObject::Array(annots));
    let input = write_incremental_update(
        reader,
        vec![IncrementalObject {
            number: p.object_number,
            generation: p.generation_number,
            object: PdfObject::Dictionary(page),
        }],
    )
    .unwrap();
    let e = ContentEngine::open_bytes(input.clone()).unwrap();
    let (xfdf, _) = export_annotation_xfdf(&e).unwrap();
    let mut selected = parse_annotation_xfdf(&xfdf).unwrap();
    selected
        .annotations
        .retain(|a| a.pdf_name.as_deref() == Some("direct-note"));
    let (output, report) = import_annotation_xfdf_pdf(
        &input,
        write_annotation_xfdf(&selected).as_bytes(),
        &preserve(),
    )
    .unwrap();
    assert_eq!(report.updated, 1);
    assert_eq!(report.created, 0);
    assert_eq!(
        report.relationship_transaction.promoted_source_ids,
        vec!["direct-note"]
    );
    let document = PdfDocument::open_bytes(output).unwrap();
    let identities = crate::annotation_identity::index(&document, MAX_ANNOTATIONS).unwrap();
    assert_eq!(identities.len(), 5);
    assert!(identities
        .values()
        .find(|i| i.id == "direct-note")
        .unwrap()
        .reference
        .is_some());
}

#[test]
fn native_geometry_preserves_actions_shared_appearance_and_relative_line_parameters() {
    let input = fixture();
    let doc = PdfDocument::open_bytes(input).unwrap();
    let ids = crate::annotation_identity::index(&doc, MAX_ANNOTATIONS).unwrap();
    let line = ids
        .values()
        .find(|i| i.page == 2 && i.name.is_none())
        .unwrap();
    let line_ref = line.reference.unwrap();
    let other = ids
        .values()
        .find(|i| i.page == 2 && i.name.is_some())
        .unwrap()
        .reference
        .unwrap();
    let reader = doc.reader();
    let n = reader.object_ids().iter().map(|r| r.0).max().unwrap() + 1;
    let mut objects = Vec::new();
    for reference in [line_ref, other] {
        let mut d = reader
            .get_object(reference.0, reference.1)
            .unwrap()
            .as_dict()
            .unwrap()
            .clone();
        let mut ap = PdfDictionary::empty();
        ap.insert("N", r(n));
        d.insert("AP", PdfObject::Dictionary(ap));
        if reference == line_ref {
            d.insert("A", r(n + 1));
            let mut aa = PdfDictionary::empty();
            aa.insert("E", r(n + 1));
            d.insert("AA", PdfObject::Dictionary(aa));
            d.insert(
                "PrivateVendorData",
                PdfObject::String(b"preserve exactly".to_vec()),
            );
            d.insert("LL", PdfObject::Real(3.0));
            d.insert("LLE", PdfObject::Real(2.0));
            d.insert("LLO", PdfObject::Real(1.0));
            d.insert(
                "CO",
                PdfObject::Array(vec![PdfObject::Real(2.0), PdfObject::Real(-1.0)]),
            );
        }
        objects.push(IncrementalObject {
            number: reference.0,
            generation: reference.1,
            object: PdfObject::Dictionary(d),
        });
    }
    let mut stream = PdfDictionary::empty();
    stream.insert("Type", PdfObject::Name("XObject".into()));
    stream.insert("Subtype", PdfObject::Name("Form".into()));
    stream.insert("BBox", rect([0.0, 0.0, 20.0, 30.0]));
    stream.insert("Resources", PdfObject::Dictionary(PdfDictionary::empty()));
    objects.push(IncrementalObject {
        number: n,
        generation: 0,
        object: PdfObject::Stream {
            dict: stream,
            raw: b"0 0 m 20 30 l S\n".to_vec(),
        },
    });
    let mut action = PdfDictionary::empty();
    action.insert("S", PdfObject::Name("URI".into()));
    action.insert(
        "URI",
        PdfObject::String(b"https://example.invalid/annotation".to_vec()),
    );
    objects.push(IncrementalObject {
        number: n + 1,
        generation: 0,
        object: PdfObject::Dictionary(action),
    });
    let input = write_incremental_update(reader, objects).unwrap();
    let before = PdfDocument::open_bytes(input.clone()).unwrap();
    let identities = crate::annotation_identity::index(&before, MAX_ANNOTATIONS).unwrap();
    let id = identities
        .values()
        .find(|i| i.reference == Some(line_ref))
        .unwrap()
        .id
        .clone();
    let (output, report) =
        move_resize_annotation_pdf(&input, &id, 1, [15.0, 40.0, 55.0, 100.0]).unwrap();
    assert_eq!(report.writer, "native_incremental_geometry_transaction");
    assert_eq!(report.canonical_import.imported_annotations, 0);
    assert_eq!(report.canonical_import.appearances_regenerated, 0);
    let after = PdfDocument::open_bytes(output.clone()).unwrap();
    let old = before.reader().get_object(line_ref.0, line_ref.1).unwrap();
    let new = after.reader().get_object(line_ref.0, line_ref.1).unwrap();
    let new = new.as_dict().unwrap();
    for key in ["A", "AA", "AP", "PrivateVendorData", "Contents", "Subtype"] {
        assert_eq!(old.as_dict().unwrap().get(key), new.get(key));
    }
    for reference in [other, (n, 0), (n + 1, 0)] {
        assert_eq!(
            before
                .reader()
                .get_object(reference.0, reference.1)
                .unwrap(),
            after.reader().get_object(reference.0, reference.1).unwrap()
        );
    }
    assert_eq!(new.get("LL").and_then(PdfObject::as_number), Some(6.0));
    assert_eq!(new.get("LLE").and_then(PdfObject::as_number), Some(4.0));
    assert_eq!(new.get("LLO").and_then(PdfObject::as_number), Some(2.0));
    assert_eq!(
        new.get("L")
            .and_then(PdfObject::as_array)
            .unwrap()
            .iter()
            .map(|v| v.as_number().unwrap())
            .collect::<Vec<_>>(),
        vec![15.0, 40.0, 55.0, 100.0]
    );
    assert_eq!(
        new.get("CO")
            .and_then(PdfObject::as_array)
            .unwrap()
            .iter()
            .map(|v| v.as_number().unwrap())
            .collect::<Vec<_>>(),
        vec![4.0, -2.0]
    );
    let sources = crate::story_anchors::annotation_anchor_sources(&output).unwrap();
    assert_eq!(
        sources.iter().find(|s| s.annotation_id == id).unwrap().page,
        1
    );
    let (again, _) =
        move_resize_annotation_pdf(&output, &id, 1, [20.0, 45.0, 60.0, 105.0]).unwrap();
    assert!(crate::story_anchors::annotation_anchor_sources(&again)
        .unwrap()
        .iter()
        .any(|s| s.annotation_id == id));
    // Anisotropic leaders cannot be represented by the same scalar fields.
    assert!(move_resize_annotation_pdf(&input, &id, 1, [15.0, 40.0, 55.0, 70.0]).is_err());
}

#[test]
fn native_batch_requires_complete_group_common_transform_and_current_revision() {
    let input = fixture();
    let sources = crate::story_anchors::annotation_anchor_sources(&input).unwrap();
    let root = sources
        .iter()
        .find(|s| s.page == 1 && s.subtype == "Text")
        .unwrap();
    let changes = sources
        .iter()
        .filter(|s| s.page == 1)
        .map(|s| AnnotationGeometryChange {
            annotation_id: s.annotation_id.clone(),
            page: 1,
            rect: s.rect.map(|v| v * 1.5 + 5.0),
        })
        .collect::<Vec<_>>();
    assert!(move_resize_annotation_pdf(&input, &root.annotation_id, 1, changes[0].rect).is_err());
    let hash = resource_digest(&input);
    let (output, report) = edit_annotation_geometries_pdf(&input, Some(&hash), &changes).unwrap();
    assert_eq!(report.changed_pages, vec![1]);
    assert_eq!(report.edits.len(), 2);
    let saved = crate::story_anchors::annotation_anchor_sources(&output).unwrap();
    for change in &changes {
        let source = saved
            .iter()
            .find(|s| s.annotation_id == change.annotation_id)
            .unwrap();
        assert_eq!(source.rect, change.rect);
        assert_eq!(
            source.group.as_ref().unwrap().topology_sha256,
            root.group.as_ref().unwrap().topology_sha256
        );
    }
    assert!(edit_annotation_geometries_pdf(&output, Some(&hash), &changes).is_err());
    let mut split = changes.clone();
    split[1].rect[0] += 1.0;
    split[1].rect[2] += 1.0;
    assert!(edit_annotation_geometries_pdf(&input, Some(&hash), &split).is_err());
    let mut conflict = changes.clone();
    for change in &mut conflict {
        change.page = 2;
    }
    assert!(edit_annotation_geometries_pdf(&input, Some(&hash), &conflict).is_err());
    let action = crate::document_subsystems::DocumentSubsystemsAction::AnnotationGeometryBatch {
        source_sha256: hash,
        changes,
    };
    let encoded = serde_json::to_value(&action).unwrap();
    assert!(
        serde_json::from_value::<crate::document_subsystems::DocumentSubsystemsAction>(encoded)
            .is_ok()
    );
    let request = crate::document_subsystems::DocumentSubsystemsRequest {
        subsystem: crate::document_subsystems::DocumentSubsystemsSubsystem::AnnotationAppearance,
        action: Some(action),
        reflow: None,
        approved: true,
        form_data: None,
        form_data_format: None,
        use_semantic_document_flow: false,
    };
    let (routed, routed_report) =
        crate::document_subsystems::apply_document_subsystems(&input, &request).unwrap();
    assert_eq!(routed_report.operation, "annotation_geometry_native_batch");
    assert_eq!(routed, output);
}

#[test]
fn native_widget_resize_preserves_field_owner_and_scales_insets_as_distances() {
    let doc = PdfDocument::open_bytes(fixture()).unwrap();
    let reader = doc.reader();
    let source = crate::annotation_identity::index(&doc, MAX_ANNOTATIONS)
        .unwrap()
        .into_values()
        .find(|s| s.page == 2 && s.name.is_none())
        .unwrap();
    let reference = source.reference.unwrap();
    let mut d = reader
        .get_object(reference.0, reference.1)
        .unwrap()
        .as_dict()
        .unwrap()
        .clone();
    d.insert("Subtype", PdfObject::Name("Widget".into()));
    d.remove("L");
    let field = reader.object_ids().iter().map(|r| r.0).max().unwrap() + 1;
    d.insert("Parent", r(field));
    d.insert("NM", PdfObject::Null);
    // Valid PDF rectangles may name opposite corners in reverse order.
    d.insert("Rect", rect([105.0, 50.0, 85.0, 20.0]));
    d.insert("RD", rect([1.0, 2.0, 3.0, 4.0]));
    let mut parent = PdfDictionary::empty();
    parent.insert("FT", PdfObject::Name("Tx".into()));
    parent.insert("Kids", PdfObject::Array(vec![r(reference.0)]));
    parent.insert("T", pdf_text_string("field"));
    parent.insert("V", pdf_text_string("unchanged"));
    let mut catalog = doc.get_catalog().unwrap();
    let mut form = PdfDictionary::empty();
    form.insert("Fields", PdfObject::Array(vec![r(field)]));
    catalog.insert("AcroForm", PdfObject::Dictionary(form));
    let root = reader.root_reference().unwrap();
    let input = write_incremental_update(
        reader,
        vec![
            IncrementalObject {
                number: reference.0,
                generation: reference.1,
                object: PdfObject::Dictionary(d),
            },
            IncrementalObject {
                number: field,
                generation: 0,
                object: PdfObject::Dictionary(parent),
            },
            IncrementalObject {
                number: root.0,
                generation: root.1,
                object: PdfObject::Dictionary(catalog),
            },
        ],
    )
    .unwrap();
    let before = PdfDocument::open_bytes(input.clone()).unwrap();
    let id = crate::annotation_identity::index(&before, MAX_ANNOTATIONS)
        .unwrap()
        .into_values()
        .find(|s| s.reference == Some(reference))
        .unwrap()
        .id;
    let (output, _) =
        move_resize_annotation_pdf(&input, &id, 2, [20.0, 20.0, 60.0, 110.0]).unwrap();
    let saved = PdfDocument::open_bytes(output).unwrap();
    let object = saved.reader().get_object(reference.0, reference.1).unwrap();
    let d = object.as_dict().unwrap();
    assert_eq!(
        d.get("RD")
            .and_then(PdfObject::as_array)
            .unwrap()
            .iter()
            .map(|v| v.as_number().unwrap())
            .collect::<Vec<_>>(),
        vec![2.0, 6.0, 6.0, 12.0]
    );
    assert_eq!(d.get_reference("Parent"), Some((field, 0)));
    assert_eq!(d.get("NM"), Some(&PdfObject::Null));
    assert_eq!(
        before.reader().get_object(field, 0).unwrap(),
        saved.reader().get_object(field, 0).unwrap()
    );
}

#[test]
fn native_move_reserves_names_of_direct_and_geometryless_destination_owners() {
    for direct in [true, false] {
        let doc = PdfDocument::open_bytes(fixture()).unwrap();
        let reader = doc.reader();
        let identities = crate::annotation_identity::index(&doc, MAX_ANNOTATIONS).unwrap();
        let line = identities
            .values()
            .find(|i| i.page == 2 && i.name.is_none())
            .unwrap()
            .reference
            .unwrap();
        let mut source = reader
            .get_object(line.0, line.1)
            .unwrap()
            .as_dict()
            .unwrap()
            .clone();
        source.insert("NM", pdf_text_string("reserved-name"));
        let page = doc.get_page(1).unwrap();
        let mut page_dict = reader
            .get_object(page.object_number, page.generation_number)
            .unwrap()
            .as_dict()
            .unwrap()
            .clone();
        let mut annots = reader
            .resolve(page_dict.get("Annots").unwrap().clone())
            .unwrap()
            .as_array()
            .unwrap()
            .to_vec();
        let mut resident = PdfDictionary::empty();
        resident.insert("Type", PdfObject::Name("Annot".into()));
        resident.insert("Subtype", PdfObject::Name("Text".into()));
        resident.insert("NM", pdf_text_string("reserved-name"));
        let mut updates = vec![IncrementalObject {
            number: line.0,
            generation: line.1,
            object: PdfObject::Dictionary(source),
        }];
        if direct {
            resident.insert("Rect", rect([120.0, 20.0, 140.0, 50.0]));
            annots.push(PdfObject::Dictionary(resident));
        } else {
            let n = reader.object_ids().iter().map(|r| r.0).max().unwrap() + 1;
            annots.push(r(n));
            updates.push(IncrementalObject {
                number: n,
                generation: 0,
                object: PdfObject::Dictionary(resident),
            });
        }
        page_dict.insert("Annots", PdfObject::Array(annots));
        updates.push(IncrementalObject {
            number: page.object_number,
            generation: page.generation_number,
            object: PdfObject::Dictionary(page_dict),
        });
        let input = write_incremental_update(reader, updates).unwrap();
        let input_doc = PdfDocument::open_bytes(input.clone()).unwrap();
        let id = crate::annotation_identity::index(&input_doc, MAX_ANNOTATIONS)
            .unwrap()
            .into_values()
            .find(|i| i.reference == Some(line))
            .unwrap()
            .id;
        assert!(move_resize_annotation_pdf(&input, &id, 1, [100.0, 60.0, 120.0, 90.0]).is_err());
        // Same-page translation is still allowed and retains the original NM.
        move_resize_annotation_pdf(&input, &id, 2, [100.0, 60.0, 120.0, 90.0]).unwrap();
    }
}

#[test]
fn appearance_preservation_requires_selected_state_and_nonsingular_finite_mapping() {
    let doc = PdfDocument::open_bytes(fixture()).unwrap();
    let reader = doc.reader();
    let identity = crate::annotation_identity::index(&doc, MAX_ANNOTATIONS)
        .unwrap()
        .into_values()
        .find(|i| i.page == 2 && i.name.is_none())
        .unwrap();
    let reference = identity.reference.unwrap();
    let mut source = reader
        .get_object(reference.0, reference.1)
        .unwrap()
        .as_dict()
        .unwrap()
        .clone();
    let n = reader.object_ids().iter().map(|r| r.0).max().unwrap() + 1;
    let mut form = PdfDictionary::empty();
    form.insert("Subtype", PdfObject::Name("Form".into()));
    form.insert("Type", PdfObject::Name("XObject".into()));
    form.insert("BBox", rect([0.0, 0.0, 20.0, 30.0]));
    assert!(valid_appearance_mapping(reader, &form));
    let mut bad = form.clone();
    bad.insert("BBox", rect([0.0, 0.0, 0.0, 30.0]));
    assert!(!valid_appearance_mapping(reader, &bad));
    bad = form.clone();
    bad.insert("Matrix", pdf_number_array(&[1.0, 0.0, 1.0, 0.0, 0.0, 0.0]));
    assert!(!valid_appearance_mapping(reader, &bad));
    bad.insert("Matrix", pdf_number_array(&[0.0, 1.0, -1.0, 0.0, 3.0, 4.0]));
    assert!(valid_appearance_mapping(reader, &bad));
    let mut states = PdfDictionary::empty();
    states.insert("Available", r(n));
    let mut ap = PdfDictionary::empty();
    ap.insert("N", PdfObject::Dictionary(states));
    source.insert("AP", PdfObject::Dictionary(ap));
    source.insert("AS", PdfObject::Name("Missing".into()));
    let input = write_incremental_update(
        reader,
        vec![
            IncrementalObject {
                number: reference.0,
                generation: reference.1,
                object: PdfObject::Dictionary(source),
            },
            IncrementalObject {
                number: n,
                generation: 0,
                object: PdfObject::Stream {
                    dict: form,
                    raw: b"0 0 m 20 30 l S\n".to_vec(),
                },
            },
        ],
    )
    .unwrap();
    let before = PdfDocument::open_bytes(input.clone()).unwrap();
    let object = before
        .reader()
        .get_object(reference.0, reference.1)
        .unwrap();
    let mut dict = object.as_dict().unwrap().clone();
    assert!(!normal_appearance_is_valid(before.reader(), &dict));
    dict.insert("AS", PdfObject::Name("Available".into()));
    assert!(normal_appearance_is_valid(before.reader(), &dict));
    dict.remove("AS");
    assert!(!normal_appearance_is_valid(before.reader(), &dict));
    let id = crate::annotation_identity::index(&before, MAX_ANNOTATIONS)
        .unwrap()
        .into_values()
        .find(|i| i.reference == Some(reference))
        .unwrap()
        .id;
    let (output, report) = generate_annotation_appearances_pdf(
        &input,
        &AnnotationAppearanceOptions {
            policy: AnnotationAppearancePolicy::RegenerateMissingOrMalformed,
            ..Default::default()
        },
    )
    .unwrap();
    let row = report.rows.iter().find(|r| r.annotation_id == id).unwrap();
    assert_eq!(row.result, "generated");
    assert_eq!(row.previous_appearance, "missing_or_malformed");
    let saved = PdfDocument::open_bytes(output).unwrap();
    let identity = crate::annotation_identity::index(&saved, MAX_ANNOTATIONS)
        .unwrap()
        .into_values()
        .find(|i| i.id == id)
        .unwrap();
    let r = identity.reference.unwrap();
    let object = saved.reader().get_object(r.0, r.1).unwrap();
    assert!(normal_appearance_is_valid(
        saved.reader(),
        object.as_dict().unwrap()
    ));
}
