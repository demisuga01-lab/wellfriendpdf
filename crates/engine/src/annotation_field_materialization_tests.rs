//! Unexecuted end-to-end source regressions for direct field dependency closure.
use super::*;
struct TreeFixture {
    base: Fixture,
    bytes: Vec<u8>,
    branch: Ref,
    branch_widget: Ref,
    shared_kids: Ref,
    selected: String,
    sibling: String,
}
fn tree_fixture() -> TreeFixture {
    let base = fixture(true, false, false, false);
    let document = PdfDocument::open_bytes(base.bytes.clone()).unwrap();
    let next = document
        .reader()
        .object_ids()
        .iter()
        .map(|r| r.0)
        .max()
        .unwrap()
        + 1;
    let branch = (next, 0);
    let branch_widget = (next + 1, 0);
    let shared_kids = (next + 2, 0);
    let mut first = dict(&document, base.shadow);
    for key in ["T", "FT", "V", "DV"] {
        first.remove(key);
    }
    let mut second = first.clone();
    second.remove("NM");
    second.insert(
        "Rect",
        PdfObject::Array(
            vec![50, 20, 70, 40]
                .into_iter()
                .map(PdfObject::Integer)
                .collect(),
        ),
    );
    let mut hidden = PdfDictionary::empty();
    hidden.insert("T", annotation_identity::text_string("hidden"));
    hidden.insert("FT", PdfObject::Name("Tx".into()));
    hidden.insert("V", annotation_identity::text_string("keep hidden"));
    let mut middle = PdfDictionary::empty();
    middle.insert("T", annotation_identity::text_string("account"));
    middle.insert("FT", PdfObject::Name("Tx".into()));
    middle.insert("V", annotation_identity::text_string("keep account"));
    middle.insert(
        "Kids",
        PdfObject::Array(vec![
            PdfObject::Dictionary(first.clone()),
            PdfObject::Dictionary(second.clone()),
            PdfObject::Dictionary(hidden),
        ]),
    );
    let mut other = first.clone();
    other.insert("Parent", reference(branch));
    other.insert("NM", annotation_identity::text_string("untouched-widget"));
    let mut branch_dict = PdfDictionary::empty();
    branch_dict.insert("T", annotation_identity::text_string("stable-branch"));
    branch_dict.insert("Kids", reference(shared_kids));
    branch_dict.insert("WFKeep", PdfObject::Integer(19));
    let mut root = PdfDictionary::empty();
    root.insert("T", annotation_identity::text_string("customer"));
    root.insert(
        "Kids",
        PdfObject::Array(vec![PdfObject::Dictionary(middle), reference(branch)]),
    );
    let mut catalog = document.get_catalog().unwrap();
    let mut form = catalog.get_dict("AcroForm").unwrap().clone();
    form.insert(
        "Fields",
        PdfObject::Array(vec![PdfObject::Dictionary(root)]),
    );
    catalog.insert("AcroForm", PdfObject::Dictionary(form));
    catalog.insert("WFSharedArray", reference(shared_kids));
    let mut page = dict(&document, base.page);
    page.insert(
        "Annots",
        PdfObject::Array(vec![
            PdfObject::Dictionary(first),
            PdfObject::Dictionary(second),
            reference(branch_widget),
        ]),
    );
    let bytes = write_objects(
        document.reader(),
        BTreeMap::from([
            (base.catalog, PdfObject::Dictionary(catalog)),
            (base.page, PdfObject::Dictionary(page)),
            (branch, PdfObject::Dictionary(branch_dict)),
            (branch_widget, PdfObject::Dictionary(other)),
            (
                shared_kids,
                PdfObject::Array(vec![reference(branch_widget)]),
            ),
        ]),
    );
    let ids = annotation_identity::index(&PdfDocument::open_bytes(bytes.clone()).unwrap(), 100_000)
        .unwrap();
    TreeFixture {
        base,
        bytes,
        branch,
        branch_widget,
        shared_kids,
        selected: ids[&(1, 0)].id.clone(),
        sibling: ids[&(1, 1)].id.clone(),
    }
}
fn assert_tree(output: &[u8], f: &TreeFixture) -> (Ref, Ref) {
    let document = PdfDocument::open_bytes(output.to_vec()).unwrap();
    let root = fields(&document)[0].as_reference().unwrap();
    let root_dict = dict(&document, root);
    assert_eq!(
        root_dict.get("T"),
        Some(&annotation_identity::text_string("customer"))
    );
    let root_kids = root_dict.get_array("Kids").unwrap();
    assert_eq!(root_kids[1], reference(f.branch));
    let middle = root_kids[0].as_reference().unwrap();
    let middle_dict = dict(&document, middle);
    assert_eq!(middle_dict.get_reference("Parent"), Some(root));
    assert_eq!(
        middle_dict.get("V"),
        Some(&annotation_identity::text_string("keep account"))
    );
    let kids = middle_dict.get_array("Kids").unwrap();
    assert_eq!(kids.len(), 3);
    let first = kids[0].as_reference().unwrap();
    let second = kids[1].as_reference().unwrap();
    let hidden = kids[2].as_reference().unwrap();
    for child in [first, second, hidden] {
        assert_eq!(dict(&document, child).get_reference("Parent"), Some(middle));
    }
    assert_eq!(
        dict(&document, hidden).get("V"),
        Some(&annotation_identity::text_string("keep hidden"))
    );
    let branch = dict(&document, f.branch);
    assert_eq!(branch.get_reference("Parent"), Some(root));
    assert_eq!(branch.get_reference("Kids"), Some(f.shared_kids));
    assert_eq!(branch.get_integer("WFKeep"), Some(19));
    assert_eq!(
        dict(&document, f.branch_widget).get_reference("Parent"),
        Some(f.branch)
    );
    assert_eq!(
        document
            .reader()
            .get_object(f.shared_kids.0, f.shared_kids.1)
            .unwrap(),
        PdfObject::Array(vec![reference(f.branch_widget)])
    );
    let ids = annotation_identity::index(&document, 100_000).unwrap();
    assert_eq!(
        ids.values().find(|i| i.id == f.selected).unwrap().reference,
        Some(first)
    );
    assert_eq!(
        ids.values().find(|i| i.id == f.sibling).unwrap().reference,
        Some(second)
    );
    (first, second)
}

#[test]
fn nested_direct_fields_include_sibling_aliases_and_preserve_indirect_subtrees() {
    let f = tree_fixture();
    let (output, report) =
        promote_annotation_sources_pdf(&f.bytes, &digest(&f.bytes), &[f.selected.clone()]).unwrap();
    assert_tree(&output, &f);
    assert_eq!(report.materialized_field_nodes, 3);
    assert_eq!(report.repaired_field_parents, 5);
    assert_eq!(
        report
            .dependent_widget_ids
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([f.selected.clone(), f.sibling.clone()])
    );
    assert_eq!(report.promoted_ids.len(), 2);
    assert!(report.field_ownership_verified);
    let (again, repeated) =
        promote_annotation_sources_pdf(&output, &digest(&output), &[f.selected.clone()]).unwrap();
    assert_eq!(again, output);
    assert_eq!(repeated.materialized_field_nodes, 0);
}

#[test]
fn native_move_does_not_move_the_sibling_needed_for_parent_repair() {
    let f = tree_fixture();
    let (output, report) = edit_annotation_geometries_pdf(
        &f.bytes,
        Some(&digest(&f.bytes)),
        &[AnnotationGeometryChange {
            annotation_id: f.selected.clone(),
            page: 2,
            rect: [80.0, 90.0, 100.0, 110.0],
        }],
    )
    .unwrap();
    let (first, second) = assert_tree(&output, &f);
    let document = PdfDocument::open_bytes(output.clone()).unwrap();
    let target = document.get_page(2).unwrap();
    assert_eq!(
        dict(&document, first).get_reference("P"),
        Some((target.object_number, target.generation_number))
    );
    assert_eq!(
        dict(&document, second).get_reference("P"),
        Some(f.base.page)
    );
    assert_eq!(
        dict(&document, second).get_array("Rect").unwrap(),
        &[
            PdfObject::Integer(50),
            PdfObject::Integer(20),
            PdfObject::Integer(70),
            PdfObject::Integer(40)
        ]
    );
    assert_eq!(report.edits.len(), 1);
    let (again, _) = edit_annotation_geometries_pdf(
        &output,
        Some(&digest(&output)),
        &[AnnotationGeometryChange {
            annotation_id: f.sibling.clone(),
            page: 1,
            rect: [60.0, 30.0, 80.0, 50.0],
        }],
    )
    .unwrap();
    assert_tree(&again, &f);
}

fn indirect_widget_with_direct_parent() -> Fixture {
    let mut f = fixture(true, true, false, false);
    let document = PdfDocument::open_bytes(f.bytes.clone()).unwrap();
    let mut widget = dict(&document, f.shadow);
    for key in ["T", "FT", "V", "DV"] {
        widget.remove(key);
    }
    let mut parent = PdfDictionary::empty();
    parent.insert("T", annotation_identity::text_string("root"));
    parent.insert("Kids", PdfObject::Array(vec![reference(f.shadow)]));
    let mut catalog = document.get_catalog().unwrap();
    let mut form = catalog.get_dict("AcroForm").unwrap().clone();
    form.insert(
        "Fields",
        PdfObject::Array(vec![PdfObject::Dictionary(parent)]),
    );
    catalog.insert("AcroForm", PdfObject::Dictionary(form));
    let mut page = dict(&document, f.page);
    page.insert("Annots", PdfObject::Array(vec![reference(f.shadow)]));
    f.bytes = write_objects(
        document.reader(),
        BTreeMap::from([
            (f.shadow, PdfObject::Dictionary(widget)),
            (f.catalog, PdfObject::Dictionary(catalog)),
            (f.page, PdfObject::Dictionary(page)),
        ]),
    );
    f
}

#[test]
fn indirect_page_widget_also_triggers_direct_ancestor_normalization() {
    let f = indirect_widget_with_direct_parent();
    let (output, report) = edit_annotation_geometries_pdf(
        &f.bytes,
        Some(&digest(&f.bytes)),
        &[AnnotationGeometryChange {
            annotation_id: "owned-direct".into(),
            page: 1,
            rect: [50.0, 60.0, 70.0, 80.0],
        }],
    )
    .unwrap();
    assert!(report.promoted_source_ids.is_empty());
    let document = PdfDocument::open_bytes(output).unwrap();
    let parent = fields(&document)[0].as_reference().unwrap();
    assert_eq!(
        dict(&document, f.shadow).get_reference("Parent"),
        Some(parent)
    );
    assert_eq!(
        dict(&document, parent).get_array("Kids").unwrap(),
        &[reference(f.shadow)]
    );
}

#[test]
fn direct_merged_field_copy_is_bound_to_its_existing_indirect_page_widget() {
    let f = fixture(true, false, false, false);
    let document = PdfDocument::open_bytes(f.bytes.clone()).unwrap();
    let mut page = dict(&document, f.page);
    page.insert("Annots", PdfObject::Array(vec![reference(f.shadow)]));
    let input = write_objects(
        document.reader(),
        BTreeMap::from([(f.page, PdfObject::Dictionary(page))]),
    );
    let (output, _) = edit_annotation_geometries_pdf(
        &input,
        Some(&digest(&input)),
        &[AnnotationGeometryChange {
            annotation_id: "owned-direct".into(),
            page: 1,
            rect: [50.0, 60.0, 70.0, 80.0],
        }],
    )
    .unwrap();
    let document = PdfDocument::open_bytes(output).unwrap();
    assert_eq!(fields(&document), vec![reference(f.shadow)]);
}

#[test]
fn contradictory_parent_and_ambiguous_sibling_are_rejected_before_returning_a_candidate() {
    let f = tree_fixture();
    let document = PdfDocument::open_bytes(f.bytes.clone()).unwrap();
    let mut catalog = document.get_catalog().unwrap();
    let mut form = catalog.get_dict("AcroForm").unwrap().clone();
    let mut root = form.get_array("Fields").unwrap()[0]
        .as_dict()
        .unwrap()
        .clone();
    let mut children = root.get_array("Kids").unwrap().to_vec();
    let mut middle = children[0].as_dict().unwrap().clone();
    middle.insert("Parent", reference(f.branch));
    children[0] = PdfObject::Dictionary(middle);
    root.insert("Kids", PdfObject::Array(children));
    form.insert(
        "Fields",
        PdfObject::Array(vec![PdfObject::Dictionary(root)]),
    );
    catalog.insert("AcroForm", PdfObject::Dictionary(form));
    let contradictory = write_objects(
        document.reader(),
        BTreeMap::from([(f.base.catalog, PdfObject::Dictionary(catalog))]),
    );
    assert!(promote_annotation_sources_pdf(
        &contradictory,
        &digest(&contradictory),
        &[f.selected.clone()]
    )
    .unwrap_err()
    .to_string()
    .contains("Parent disagrees"));
    let mut page = dict(&document, f.base.page);
    let mut annotations = page.get_array("Annots").unwrap().to_vec();
    annotations.push(annotations[1].clone());
    page.insert("Annots", PdfObject::Array(annotations));
    let ambiguous = write_objects(
        document.reader(),
        BTreeMap::from([(f.base.page, PdfObject::Dictionary(page))]),
    );
    assert!(
        promote_annotation_sources_pdf(&ambiguous, &digest(&ambiguous), &[f.selected.clone()])
            .unwrap_err()
            .to_string()
            .contains("ambiguous page widget")
    );
}

#[test]
fn an_intermediate_revision_cannot_change_a_direct_ancestor_silently() {
    let f = tree_fixture();
    let document = PdfDocument::open_bytes(f.bytes.clone()).unwrap();
    let mut catalog = document.get_catalog().unwrap();
    let mut form = catalog.get_dict("AcroForm").unwrap().clone();
    let mut root = form.get_array("Fields").unwrap()[0]
        .as_dict()
        .unwrap()
        .clone();
    root.insert("T", annotation_identity::text_string("changed-root"));
    form.insert(
        "Fields",
        PdfObject::Array(vec![PdfObject::Dictionary(root)]),
    );
    catalog.insert("AcroForm", PdfObject::Dictionary(form));
    let staged = write_objects(
        document.reader(),
        BTreeMap::from([(f.base.catalog, PdfObject::Dictionary(catalog))]),
    );
    assert!(
        stage(&f.bytes, &staged, &BTreeSet::from([f.selected]), 100_000)
            .unwrap_err()
            .to_string()
            .contains("ancestor changed")
    );
}

#[test]
fn geometry_invalidation_includes_a_sibling_on_another_page_without_moving_it() {
    let f = tree_fixture();
    let document = PdfDocument::open_bytes(f.bytes.clone()).unwrap();
    let second_page = document.get_page(2).unwrap();
    let second_ref = (second_page.object_number, second_page.generation_number);
    let mut first_page = dict(&document, f.base.page);
    let mut annotations = first_page.get_array("Annots").unwrap().to_vec();
    let mut sibling = annotations.remove(1).as_dict().unwrap().clone();
    sibling.insert("P", reference(second_ref));
    first_page.insert("Annots", PdfObject::Array(annotations));
    let mut second_page_dict = dict(&document, second_ref);
    second_page_dict.insert(
        "Annots",
        PdfObject::Array(vec![PdfObject::Dictionary(sibling.clone())]),
    );
    let mut catalog = document.get_catalog().unwrap();
    let mut form = catalog.get_dict("AcroForm").unwrap().clone();
    let mut root = form.get_array("Fields").unwrap()[0]
        .as_dict()
        .unwrap()
        .clone();
    let mut children = root.get_array("Kids").unwrap().to_vec();
    let mut middle = children[0].as_dict().unwrap().clone();
    let mut kids = middle.get_array("Kids").unwrap().to_vec();
    kids[1] = PdfObject::Dictionary(sibling);
    middle.insert("Kids", PdfObject::Array(kids));
    children[0] = PdfObject::Dictionary(middle);
    root.insert("Kids", PdfObject::Array(children));
    form.insert(
        "Fields",
        PdfObject::Array(vec![PdfObject::Dictionary(root)]),
    );
    catalog.insert("AcroForm", PdfObject::Dictionary(form));
    let input = write_objects(
        document.reader(),
        BTreeMap::from([
            (f.base.page, PdfObject::Dictionary(first_page)),
            (second_ref, PdfObject::Dictionary(second_page_dict)),
            (f.base.catalog, PdfObject::Dictionary(catalog)),
        ]),
    );
    let (output, report) = edit_annotation_geometries_pdf(
        &input,
        Some(&digest(&input)),
        &[AnnotationGeometryChange {
            annotation_id: f.selected,
            page: 1,
            rect: [20.0, 30.0, 40.0, 50.0],
        }],
    )
    .unwrap();
    assert_eq!(report.changed_pages, vec![1, 2]);
    assert_eq!(report.edits[0].affected_pages, vec![1, 2]);
    let document = PdfDocument::open_bytes(output).unwrap();
    let page = dict(&document, second_ref);
    let sibling = dict(
        &document,
        page.get_array("Annots").unwrap()[0].as_reference().unwrap(),
    );
    assert_eq!(sibling.get_reference("P"), Some(second_ref));
    assert_eq!(
        sibling.get_array("Rect").unwrap(),
        &[
            PdfObject::Integer(50),
            PdfObject::Integer(20),
            PdfObject::Integer(70),
            PdfObject::Integer(40)
        ]
    );
}

#[test]
fn direct_field_ancestor_and_tag_owner_keep_one_authoritative_widget() {
    let f = fixture(true, true, true, true);
    let document = PdfDocument::open_bytes(f.bytes.clone()).unwrap();
    let mut widget = dict(&document, f.shadow);
    let mut parent = PdfDictionary::empty();
    for key in ["T", "FT", "V", "DV"] {
        if let Some(value) = widget.remove(key) {
            parent.insert(key, value);
        }
    }
    parent.insert("Kids", PdfObject::Array(vec![reference(f.shadow)]));
    let mut page = dict(&document, f.page);
    page.insert(
        "Annots",
        PdfObject::Array(vec![PdfObject::Dictionary(widget.clone())]),
    );
    let mut catalog = document.get_catalog().unwrap();
    let mut form = catalog.get_dict("AcroForm").unwrap().clone();
    form.insert(
        "Fields",
        PdfObject::Array(vec![PdfObject::Dictionary(parent)]),
    );
    catalog.insert("AcroForm", PdfObject::Dictionary(form));
    let input = write_objects(
        document.reader(),
        BTreeMap::from([
            (f.shadow, PdfObject::Dictionary(widget)),
            (f.page, PdfObject::Dictionary(page)),
            (f.catalog, PdfObject::Dictionary(catalog)),
        ]),
    );
    let (output, report) = promote(&input).unwrap();
    assert_eq!(report.reused_owner_ids, vec!["owned-direct"]);
    assert_eq!(report.materialized_field_nodes, 1);
    assert!(report.field_ownership_verified && report.tagged_ownership_verified);
    let document = PdfDocument::open_bytes(output.clone()).unwrap();
    let parent = fields(&document)[0].as_reference().unwrap();
    assert_eq!(
        dict(&document, f.shadow).get_reference("Parent"),
        Some(parent)
    );
    assert_eq!(
        dict(&document, parent).get("V"),
        Some(&annotation_identity::text_string("existing value"))
    );
    assert_eq!(
        dict(&document, f.tag)
            .get_dict("K")
            .unwrap()
            .get_reference("Obj"),
        Some(f.shadow)
    );
    crate::tagged_structure::validate_parent_tree(&output).unwrap();
}
