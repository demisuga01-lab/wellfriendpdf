//! Unexecuted ownership regression source; no PDF workloads are authorized.
use super::*;
use crate::annotation_media_redaction::{edit_annotation_geometries_pdf, AnnotationGeometryChange};
use crate::authoring::{PageSize, PdfBuilder};
#[path = "annotation_field_materialization_tests.rs"]
mod field_tests;
#[path = "annotation_structure_materialization_tests.rs"]
mod structure_tests;

struct Fixture {
    bytes: Vec<u8>,
    shadow: Ref,
    tag: Ref,
    structure: Ref,
    page: Ref,
    catalog: Ref,
}
fn fixture(widget: bool, field_shadow: bool, tagged: bool, tag_shadow: bool) -> Fixture {
    let mut builder = PdfBuilder::new();
    builder.add_page(PageSize::custom(200.0, 200.0));
    builder.add_page(PageSize::custom(200.0, 200.0));
    let document = PdfDocument::open_bytes(builder.to_bytes().unwrap()).unwrap();
    let reader = document.reader();
    let p = document.get_page(1).unwrap();
    let page = (p.object_number, p.generation_number);
    let catalog = reader.root_reference().unwrap();
    let next = reader.object_ids().iter().map(|r| r.0).max().unwrap() + 1;
    let shadow = (next, 0);
    let structure = (next + 1, 0);
    let tag = (next + 2, 0);
    let mut annotation = PdfDictionary::empty();
    annotation.insert("Type", PdfObject::Name("Annot".into()));
    annotation.insert(
        "Subtype",
        PdfObject::Name(if widget { "Widget" } else { "Text" }.into()),
    );
    annotation.insert("NM", annotation_identity::text_string("owned-direct"));
    annotation.insert(
        "Rect",
        PdfObject::Array(
            vec![10, 20, 30, 40]
                .into_iter()
                .map(PdfObject::Integer)
                .collect(),
        ),
    );
    annotation.insert("P", reference(page));
    annotation.insert("WFOpaque", PdfObject::String(b"keep-native".to_vec()));
    if widget {
        annotation.insert("T", annotation_identity::text_string("customer"));
        annotation.insert("FT", PdfObject::Name("Tx".into()));
        annotation.insert("V", annotation_identity::text_string("existing value"));
        annotation.insert("DV", annotation_identity::text_string("default value"));
    }
    if tagged {
        annotation.insert("StructParent", PdfObject::Integer(7));
    }
    let mut page_dict = reader
        .get_object(page.0, page.1)
        .unwrap()
        .as_dict()
        .unwrap()
        .clone();
    page_dict.insert(
        "Annots",
        PdfObject::Array(vec![PdfObject::Dictionary(annotation.clone())]),
    );
    let mut catalog_dict = document.get_catalog().unwrap();
    let mut objects = BTreeMap::from([
        (page, PdfObject::Dictionary(page_dict)),
        (shadow, PdfObject::Dictionary(annotation.clone())),
    ]);
    if widget {
        let mut form = PdfDictionary::empty();
        form.insert(
            "Fields",
            PdfObject::Array(vec![if field_shadow {
                reference(shadow)
            } else {
                PdfObject::Dictionary(annotation.clone())
            }]),
        );
        form.insert("NeedAppearances", PdfObject::Boolean(false));
        catalog_dict.insert("AcroForm", PdfObject::Dictionary(form));
    }
    if tagged {
        let mut objr = PdfDictionary::empty();
        objr.insert("Type", PdfObject::Name("OBJR".into()));
        objr.insert("Pg", reference(page));
        objr.insert(
            "Obj",
            if tag_shadow {
                reference(shadow)
            } else {
                PdfObject::Dictionary(annotation)
            },
        );
        let mut owner = PdfDictionary::empty();
        owner.insert("Type", PdfObject::Name("StructElem".into()));
        owner.insert(
            "S",
            PdfObject::Name(if widget { "Form" } else { "Annot" }.into()),
        );
        owner.insert("P", reference(structure));
        owner.insert("Pg", reference(page));
        owner.insert("K", PdfObject::Dictionary(objr));
        owner.insert(
            "Alt",
            annotation_identity::text_string("original alternative"),
        );
        let mut tree = PdfDictionary::empty();
        tree.insert(
            "Nums",
            PdfObject::Array(vec![PdfObject::Integer(7), reference(tag)]),
        );
        let mut root = PdfDictionary::empty();
        root.insert("Type", PdfObject::Name("StructTreeRoot".into()));
        root.insert("K", reference(tag));
        root.insert("ParentTree", PdfObject::Dictionary(tree));
        root.insert("ParentTreeNextKey", PdfObject::Integer(8));
        objects.insert(structure, PdfObject::Dictionary(root));
        objects.insert(tag, PdfObject::Dictionary(owner));
        catalog_dict.insert("StructTreeRoot", reference(structure));
    }
    objects.insert(catalog, PdfObject::Dictionary(catalog_dict));
    Fixture {
        bytes: write_objects(reader, objects),
        shadow,
        tag,
        structure,
        page,
        catalog,
    }
}
fn write_objects(reader: &PdfReader, objects: BTreeMap<Ref, PdfObject>) -> Vec<u8> {
    write_incremental_update(
        reader,
        objects
            .into_iter()
            .map(|(r, object)| IncrementalObject {
                number: r.0,
                generation: r.1,
                object,
            })
            .collect(),
    )
    .unwrap()
}
fn dict(document: &PdfDocument, id: Ref) -> PdfDictionary {
    document
        .reader()
        .get_object(id.0, id.1)
        .unwrap()
        .as_dict()
        .unwrap()
        .clone()
}
fn promoted_ref(output: &[u8]) -> Ref {
    let document = PdfDocument::open_bytes(output.to_vec()).unwrap();
    annotation_identity::index(&document, 100_000)
        .unwrap()
        .values()
        .find(|i| i.id == "owned-direct")
        .unwrap()
        .reference
        .unwrap()
}
fn promote(input: &[u8]) -> Result<(Vec<u8>, AnnotationPromotionReport)> {
    promote_annotation_sources_pdf(input, &digest(input), &["owned-direct".into()])
}
fn fields(document: &PdfDocument) -> Vec<PdfObject> {
    let form = document
        .reader()
        .resolve(
            document
                .get_catalog()
                .unwrap()
                .get("AcroForm")
                .unwrap()
                .clone(),
        )
        .unwrap();
    document
        .reader()
        .resolve(form.as_dict().unwrap().get("Fields").unwrap().clone())
        .unwrap()
        .as_array()
        .unwrap()
        .to_vec()
}

#[test]
fn merged_direct_widget_materializes_once_for_page_and_field_without_losing_values() {
    let f = fixture(true, false, false, false);
    let (output, report) = promote(&f.bytes).unwrap();
    let reference = promoted_ref(&output);
    assert_ne!(reference, f.shadow); // Unreachable equal objects are not owners.
    let document = PdfDocument::open_bytes(output).unwrap();
    assert_eq!(fields(&document), vec![super::reference(reference)]);
    let widget = dict(&document, reference);
    assert_eq!(
        widget.get("V"),
        Some(&annotation_identity::text_string("existing value"))
    );
    assert_eq!(
        widget.get("DV"),
        Some(&annotation_identity::text_string("default value"))
    );
    assert_eq!(
        widget.get("WFOpaque"),
        Some(&PdfObject::String(b"keep-native".to_vec()))
    );
    assert!(report.field_ownership_verified);
    assert!(!report.tagged_ownership_verified);
    assert!(report.reused_owner_ids.is_empty());
}

#[test]
fn one_indirect_object_is_reused_when_field_and_tag_owners_agree() {
    let f = fixture(true, true, true, true);
    let (output, report) = promote(&f.bytes).unwrap();
    assert_eq!(promoted_ref(&output), f.shadow);
    assert_eq!(report.reused_owner_ids, vec!["owned-direct"]);
    assert!(report.field_ownership_verified && report.tagged_ownership_verified);
    let document = PdfDocument::open_bytes(output.clone()).unwrap();
    assert_eq!(fields(&document), vec![reference(f.shadow)]);
    assert_eq!(
        dict(&document, f.tag)
            .get_dict("K")
            .unwrap()
            .get_reference("Obj"),
        Some(f.shadow)
    );
    assert_eq!(
        crate::tagged_structure::validate_parent_tree(&output)
            .unwrap()
            .object_reference_items,
        1
    );
    let (moved, _) = edit_annotation_geometries_pdf(
        &output,
        Some(&digest(&output)),
        &[AnnotationGeometryChange {
            annotation_id: "owned-direct".into(),
            page: 2,
            rect: [40.0, 50.0, 60.0, 70.0],
        }],
    )
    .unwrap();
    let document = PdfDocument::open_bytes(moved.clone()).unwrap();
    let second = document.get_page(2).unwrap();
    assert_eq!(
        dict(&document, f.shadow).get_reference("P"),
        Some((second.object_number, second.generation_number))
    );
    assert_eq!(
        dict(&document, f.tag)
            .get_dict("K")
            .unwrap()
            .get_reference("Pg"),
        Some((second.object_number, second.generation_number))
    );
    assert_eq!(fields(&document), vec![reference(f.shadow)]);
    crate::tagged_structure::validate_parent_tree(&moved).unwrap();
}

#[test]
fn direct_objr_carrier_is_patched_without_changing_parent_key_or_semantic_metadata() {
    let f = fixture(false, false, true, false);
    assert!(crate::tagged_structure::validate_parent_tree(&f.bytes).is_err());
    let (output, report) = promote(&f.bytes).unwrap();
    assert!(report.tagged_ownership_verified);
    let id = promoted_ref(&output);
    let document = PdfDocument::open_bytes(output.clone()).unwrap();
    let tag = dict(&document, f.tag);
    assert_eq!(tag.get_dict("K").unwrap().get_reference("Obj"), Some(id));
    assert_eq!(
        tag.get("Alt"),
        Some(&annotation_identity::text_string("original alternative"))
    );
    assert_eq!(dict(&document, id).get_integer("StructParent"), Some(7));
    assert_eq!(
        dict(&document, f.structure)
            .get_dict("ParentTree")
            .unwrap()
            .get_array("Nums")
            .unwrap(),
        &[PdfObject::Integer(7), reference(f.tag)]
    );
    crate::tagged_structure::validate_parent_tree(&output).unwrap();
}

#[test]
fn mixed_direct_field_and_indirect_tag_owner_share_the_same_result() {
    let f = fixture(true, false, true, true);
    let (output, report) = promote(&f.bytes).unwrap();
    assert_eq!(promoted_ref(&output), f.shadow);
    assert_eq!(
        fields(&PdfDocument::open_bytes(output).unwrap()),
        vec![reference(f.shadow)]
    );
    assert!(report.field_ownership_verified && report.tagged_ownership_verified);
}

#[test]
fn conflicting_shadow_owners_and_already_page_owned_objects_are_not_merged() {
    let f = fixture(true, true, true, true);
    let doc = PdfDocument::open_bytes(f.bytes.clone()).unwrap();
    let other = (
        doc.reader().object_ids().iter().map(|r| r.0).max().unwrap() + 1,
        0,
    );
    let mut owner = dict(&doc, f.tag);
    let mut k = owner.get_dict("K").unwrap().clone();
    k.insert("Obj", reference(other));
    owner.insert("K", PdfObject::Dictionary(k));
    let conflicting = write_objects(
        doc.reader(),
        BTreeMap::from([
            (f.tag, PdfObject::Dictionary(owner)),
            (other, PdfObject::Dictionary(dict(&doc, f.shadow))),
        ]),
    );
    assert!(promote(&conflicting)
        .unwrap_err()
        .to_string()
        .contains("disagree"));
    let mut page = dict(&doc, f.page);
    let mut entries = page.get_array("Annots").unwrap().to_vec();
    entries.push(reference(f.shadow));
    page.insert("Annots", PdfObject::Array(entries));
    let already_owned = write_objects(
        doc.reader(),
        BTreeMap::from([(f.page, PdfObject::Dictionary(page))]),
    );
    let document = PdfDocument::open_bytes(already_owned.clone()).unwrap();
    let id = annotation_identity::index(&document, 100_000).unwrap()[&(1, 0)]
        .id
        .clone();
    let error = promote_annotation_sources_pdf(&already_owned, &digest(&already_owned), &[id])
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("structure carrier needs one exact page annotation occurrence"),
        "unexpected refusal: {error}"
    );
}

#[test]
fn two_widget_promotions_compose_in_one_parent_and_do_not_rewrite_a_shared_kids_array() {
    let f = fixture(true, false, false, false);
    let doc = PdfDocument::open_bytes(f.bytes.clone()).unwrap();
    let parent = (
        doc.reader().object_ids().iter().map(|r| r.0).max().unwrap() + 1,
        0,
    );
    let kids = (parent.0 + 1, 0);
    let mut first = dict(&doc, f.shadow);
    first.remove("T");
    first.remove("FT");
    first.remove("V");
    first.remove("DV");
    first.insert("Parent", reference(parent));
    let mut second = first.clone();
    second.insert("NM", annotation_identity::text_string("second-widget"));
    let children = vec![
        PdfObject::Dictionary(first.clone()),
        PdfObject::Dictionary(second.clone()),
    ];
    let mut field = PdfDictionary::empty();
    field.insert("T", annotation_identity::text_string("parent-field"));
    field.insert("FT", PdfObject::Name("Tx".into()));
    field.insert("V", annotation_identity::text_string("do not change"));
    field.insert("Kids", reference(kids));
    let mut page = dict(&doc, f.page);
    page.insert("Annots", PdfObject::Array(children.clone()));
    let mut catalog = doc.get_catalog().unwrap();
    let mut form = catalog.get_dict("AcroForm").unwrap().clone();
    form.insert("Fields", PdfObject::Array(vec![reference(parent)]));
    catalog.insert("AcroForm", PdfObject::Dictionary(form));
    catalog.insert("WFSharedKids", reference(kids));
    let input = write_objects(
        doc.reader(),
        BTreeMap::from([
            (parent, PdfObject::Dictionary(field)),
            (kids, PdfObject::Array(children.clone())),
            (f.page, PdfObject::Dictionary(page)),
            (f.catalog, PdfObject::Dictionary(catalog)),
        ]),
    );
    let (output, report) = promote_annotation_sources_pdf(
        &input,
        &digest(&input),
        &["owned-direct".into(), "second-widget".into()],
    )
    .unwrap();
    assert_eq!(report.promoted_ids.len(), 2);
    let document = PdfDocument::open_bytes(output).unwrap();
    assert_eq!(
        document.reader().get_object(kids.0, kids.1).unwrap(),
        PdfObject::Array(children)
    );
    let field = dict(&document, parent);
    assert_eq!(
        field.get("V"),
        Some(&annotation_identity::text_string("do not change"))
    );
    let children = field.get_array("Kids").unwrap();
    assert_eq!(children.len(), 2);
    for child in children {
        let id = child.as_reference().unwrap();
        assert_eq!(dict(&document, id).get_reference("Parent"), Some(parent));
    }
    assert!(report.field_ownership_verified);
}

#[test]
fn owner_mutation_in_an_intermediate_revision_invalidates_staging() {
    let f = fixture(true, true, false, false);
    let doc = PdfDocument::open_bytes(f.bytes.clone()).unwrap();
    let mut shadow = dict(&doc, f.shadow);
    shadow.insert("V", annotation_identity::text_string("changed"));
    let staged = write_objects(
        doc.reader(),
        BTreeMap::from([(f.shadow, PdfObject::Dictionary(shadow))]),
    );
    assert!(stage(
        &f.bytes,
        &staged,
        &BTreeSet::from(["owned-direct".into()]),
        100_000
    )
    .unwrap_err()
    .to_string()
    .contains("owner changed"));
}

#[test]
fn equal_page_widgets_cannot_both_claim_one_field_owner() {
    let f = fixture(true, false, false, false);
    let doc = PdfDocument::open_bytes(f.bytes.clone()).unwrap();
    let mut page = dict(&doc, f.page);
    let annotation = page.get_array("Annots").unwrap()[0].clone();
    page.insert(
        "Annots",
        PdfObject::Array(vec![annotation.clone(), annotation]),
    );
    let input = write_objects(
        doc.reader(),
        BTreeMap::from([(f.page, PdfObject::Dictionary(page))]),
    );
    let ids = annotation_identity::index(&PdfDocument::open_bytes(input.clone()).unwrap(), 100_000)
        .unwrap();
    let error = promote_annotation_sources_pdf(&input, &digest(&input), &[ids[&(1, 0)].id.clone()])
        .unwrap_err();
    assert!(error.to_string().contains("indistinguishable"));
}

#[test]
fn indirect_subtype_and_structparent_values_use_the_same_owner_and_editing_semantics() {
    let f = fixture(true, true, true, true);
    let document = PdfDocument::open_bytes(f.bytes.clone()).unwrap();
    let next = document
        .reader()
        .object_ids()
        .iter()
        .map(|r| r.0)
        .max()
        .unwrap()
        + 1;
    let mut annotation = dict(&document, f.shadow);
    annotation.insert("Subtype", reference((next, 0)));
    annotation.insert("StructParent", reference((next + 1, 0)));
    let mut page = dict(&document, f.page);
    page.insert(
        "Annots",
        PdfObject::Array(vec![PdfObject::Dictionary(annotation.clone())]),
    );
    let input = write_objects(
        document.reader(),
        BTreeMap::from([
            (f.shadow, PdfObject::Dictionary(annotation)),
            (f.page, PdfObject::Dictionary(page)),
            ((next, 0), PdfObject::Name("Widget".into())),
            ((next + 1, 0), PdfObject::Integer(7)),
        ]),
    );
    let sources = crate::story_anchors::annotation_anchor_sources(&input).unwrap();
    assert_eq!(sources[0].subtype, "Widget");
    let (output, report) = edit_annotation_geometries_pdf(
        &input,
        Some(&digest(&input)),
        &[AnnotationGeometryChange {
            annotation_id: "owned-direct".into(),
            page: 2,
            rect: [40.0, 50.0, 60.0, 70.0],
        }],
    )
    .unwrap();
    assert_eq!(report.promoted_source_ids, vec!["owned-direct"]);
    let document = PdfDocument::open_bytes(output.clone()).unwrap();
    assert_eq!(
        dict(&document, f.shadow).get_integer("StructParent"),
        Some(7)
    );
    assert_eq!(fields(&document), vec![reference(f.shadow)]);
    crate::tagged_structure::validate_parent_tree(&output).unwrap();
}
