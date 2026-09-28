//! Source regressions only: execution is deferred to the user's VPS phase.
use super::*;

fn tree(document: &PdfDocument) -> (Ref, PdfDictionary) {
    let id = document
        .get_catalog()
        .unwrap()
        .get_reference("StructTreeRoot")
        .unwrap();
    (id, dict(document, id))
}
fn entries(document: &PdfDocument, root: &PdfDictionary, key: &str, array: &str) -> Vec<PdfObject> {
    let owner = document
        .reader()
        .resolve(root.get(key).unwrap().clone())
        .unwrap();
    document
        .reader()
        .resolve(owner.as_dict().unwrap().get(array).unwrap().clone())
        .unwrap()
        .as_array()
        .unwrap()
        .to_vec()
}
fn direct_owner_input(f: &Fixture, root_direct: bool, shadow: bool) -> Vec<u8> {
    let document = PdfDocument::open_bytes(f.bytes.clone()).unwrap();
    let mut root = dict(&document, f.structure);
    let mut owner = dict(&document, f.tag);
    if root_direct {
        owner.remove("P");
    }
    owner.insert("ID", PdfObject::String(b"annotation-owner".to_vec()));
    let owner_value = if shadow {
        reference(f.tag)
    } else {
        PdfObject::Dictionary(owner.clone())
    };
    root.insert("K", PdfObject::Dictionary(owner.clone()));
    let mut parent_tree = PdfDictionary::empty();
    parent_tree.insert(
        "Nums",
        PdfObject::Array(vec![PdfObject::Integer(7), owner_value.clone()]),
    );
    parent_tree.insert("WFKeep", PdfObject::String(b"lookup metadata".to_vec()));
    root.insert("ParentTree", PdfObject::Dictionary(parent_tree));
    let mut id_tree = PdfDictionary::empty();
    id_tree.insert(
        "Names",
        PdfObject::Array(vec![
            PdfObject::String(b"annotation-owner".to_vec()),
            owner_value,
        ]),
    );
    root.insert("IDTree", PdfObject::Dictionary(id_tree));
    let mut objects = BTreeMap::from([(f.tag, PdfObject::Dictionary(owner))]);
    if root_direct {
        let mut catalog = document.get_catalog().unwrap();
        catalog.insert("StructTreeRoot", PdfObject::Dictionary(root));
        objects.insert(f.catalog, PdfObject::Dictionary(catalog));
    } else {
        objects.insert(f.structure, PdfObject::Dictionary(root));
    }
    write_objects(document.reader(), objects)
}
fn assert_owner(output: &[u8], f: &Fixture) -> (Ref, Ref) {
    let document = PdfDocument::open_bytes(output.to_vec()).unwrap();
    let (root_ref, root) = tree(&document);
    let owner_ref = root.get_reference("K").unwrap();
    let owner = dict(&document, owner_ref);
    assert_eq!(owner.get_reference("P"), Some(root_ref));
    assert_eq!(
        owner.get("Alt"),
        dict(&PdfDocument::open_bytes(f.bytes.clone()).unwrap(), f.tag).get("Alt")
    );
    assert_eq!(
        owner.get_dict("K").unwrap().get_reference("Obj"),
        Some(promoted_ref(output))
    );
    assert_eq!(
        entries(&document, &root, "ParentTree", "Nums"),
        vec![PdfObject::Integer(7), reference(owner_ref)]
    );
    assert_eq!(
        entries(&document, &root, "IDTree", "Names"),
        vec![
            PdfObject::String(b"annotation-owner".to_vec()),
            reference(owner_ref)
        ]
    );
    assert_eq!(root.get_integer("ParentTreeNextKey"), Some(8));
    assert_eq!(
        document
            .reader()
            .resolve(root.get("ParentTree").unwrap().clone())
            .unwrap()
            .as_dict()
            .unwrap()
            .get("WFKeep"),
        Some(&PdfObject::String(b"lookup metadata".to_vec()))
    );
    crate::tagged_structure::validate_parent_tree(output).unwrap();
    (root_ref, owner_ref)
}

#[test]
fn direct_owner_and_lookup_copies_get_one_identity_without_changing_lookup_keys() {
    let f = fixture(false, false, true, false);
    let input = direct_owner_input(&f, false, false);
    let (output, report) = promote(&input).unwrap();
    let (root, owner) = assert_owner(&output, &f);
    assert_eq!(root, f.structure);
    assert_ne!(owner, f.tag); // An unreachable equal dictionary is not authoritative.
    assert_eq!(report.materialized_structure_nodes, 1);
    assert_eq!(report.normalized_structure_links, 2);
    assert!(report.tagged_ownership_verified);
    let (again, repeated) = promote(&output).unwrap();
    assert_eq!(again, output);
    assert_eq!(repeated.materialized_structure_nodes, 0);
}

#[test]
fn direct_root_replacement_composes_with_staged_annotation_carrier_updates() {
    let f = fixture(false, false, true, false);
    let input = direct_owner_input(&f, true, false);
    let (output, report) = promote(&input).unwrap();
    let (root, owner) = assert_owner(&output, &f);
    assert_ne!(root, f.structure);
    assert_ne!(owner, f.tag);
    assert_eq!(report.materialized_structure_nodes, 2);
    assert_eq!(report.repaired_structure_parents, 1);
}

#[test]
fn declared_indirect_shadow_owner_is_reused_by_parent_and_id_indexes() {
    let f = fixture(false, false, true, false);
    let input = direct_owner_input(&f, false, true);
    let (output, report) = promote(&input).unwrap();
    assert_eq!(assert_owner(&output, &f), (f.structure, f.tag));
    assert_eq!(report.materialized_structure_nodes, 1);
    assert_eq!(report.normalized_structure_links, 0);
}

#[test]
fn direct_root_reuses_one_shadow_named_by_existing_parent_links() {
    let f = fixture(false, false, true, false);
    let input = direct_owner_input(&f, false, false);
    let document = PdfDocument::open_bytes(input).unwrap();
    let mut catalog = document.get_catalog().unwrap();
    catalog.insert(
        "StructTreeRoot",
        PdfObject::Dictionary(dict(&document, f.structure)),
    );
    let input = write_objects(
        document.reader(),
        BTreeMap::from([(f.catalog, PdfObject::Dictionary(catalog))]),
    );
    let (output, report) = promote(&input).unwrap();
    let (root, owner) = assert_owner(&output, &f);
    assert_eq!(root, f.structure);
    assert_ne!(owner, f.tag);
    assert_eq!(report.materialized_structure_nodes, 2);
    assert_eq!(report.repaired_structure_parents, 0);
}

#[test]
fn indirect_page_annotation_with_direct_objr_copy_triggers_native_preparation() {
    let f = fixture(false, false, true, false);
    let document = PdfDocument::open_bytes(f.bytes.clone()).unwrap();
    let mut page = dict(&document, f.page);
    page.insert("Annots", PdfObject::Array(vec![reference(f.shadow)]));
    let input = write_objects(
        document.reader(),
        BTreeMap::from([(f.page, PdfObject::Dictionary(page))]),
    );
    let (output, report) = edit_annotation_geometries_pdf(
        &input,
        Some(&digest(&input)),
        &[AnnotationGeometryChange {
            annotation_id: "owned-direct".into(),
            page: 2,
            rect: [60.0, 70.0, 80.0, 90.0],
        }],
    )
    .unwrap();
    assert_eq!(promoted_ref(&output), f.shadow);
    assert!(
        report
            .source_normalization
            .as_ref()
            .unwrap()
            .tagged_ownership_verified
    );
    let reopened = PdfDocument::open_bytes(output.clone()).unwrap();
    assert_eq!(
        dict(&reopened, f.tag)
            .get_dict("K")
            .unwrap()
            .get_reference("Obj"),
        Some(f.shadow)
    );
    crate::tagged_structure::validate_parent_tree(&output).unwrap();
}

#[test]
fn nested_direct_ancestors_preserve_mcid_arrays_and_private_array_aliases() {
    let f = fixture(false, false, true, false);
    let document = PdfDocument::open_bytes(f.bytes.clone()).unwrap();
    let next = document
        .reader()
        .object_ids()
        .iter()
        .map(|r| r.0)
        .max()
        .unwrap()
        + 1;
    let nums_ref = (next, 0);
    let content_ref = (next + 1, 0);
    let mut leaf = dict(&document, f.tag);
    leaf.remove("P");
    let mut mcr = PdfDictionary::empty(); // Type is optional for an MCR dictionary.
    mcr.insert("MCID", PdfObject::Integer(1));
    mcr.insert("Pg", reference(f.page));
    leaf.insert(
        "K",
        PdfObject::Array(vec![
            leaf.get("K").unwrap().clone(),
            PdfObject::Dictionary(mcr),
        ]),
    );
    let mut parent = PdfDictionary::empty();
    parent.insert("Type", PdfObject::Name("StructElem".into()));
    parent.insert("S", PdfObject::Name("Document".into()));
    parent.insert("K", PdfObject::Dictionary(leaf.clone()));
    let nums = PdfObject::Array(vec![
        PdfObject::Integer(7),
        PdfObject::Dictionary(leaf.clone()),
        PdfObject::Integer(9),
        PdfObject::Array(vec![PdfObject::Null, PdfObject::Dictionary(leaf)]),
    ]);
    let mut lookup = PdfDictionary::empty();
    lookup.insert("Nums", reference(nums_ref));
    let mut root = dict(&document, f.structure);
    root.insert("K", PdfObject::Dictionary(parent));
    root.insert("ParentTree", PdfObject::Dictionary(lookup));
    root.insert("ParentTreeNextKey", PdfObject::Integer(10));
    let mut catalog = document.get_catalog().unwrap();
    catalog.insert("StructTreeRoot", PdfObject::Dictionary(root));
    catalog.insert("WFPrivateArray", reference(nums_ref));
    let mut page = dict(&document, f.page);
    page.insert("StructParents", PdfObject::Integer(9));
    page.insert("Contents", reference(content_ref));
    let raw = b"/Span << /MCID 1 >> BDC 0 0 m 10 10 l S EMC\n".to_vec();
    let input = write_objects(
        document.reader(),
        BTreeMap::from([
            (f.catalog, PdfObject::Dictionary(catalog)),
            (f.page, PdfObject::Dictionary(page)),
            (nums_ref, nums.clone()),
            (
                content_ref,
                PdfObject::Stream {
                    dict: PdfDictionary::empty(),
                    raw,
                },
            ),
        ]),
    );
    let (output, report) = promote(&input).unwrap();
    let reopened = PdfDocument::open_bytes(output.clone()).unwrap();
    let (root_id, root) = tree(&reopened);
    let parent_id = root.get_reference("K").unwrap();
    let parent = dict(&reopened, parent_id);
    let leaf_id = parent.get_reference("K").unwrap();
    assert_eq!(parent.get_reference("P"), Some(root_id));
    assert_eq!(dict(&reopened, leaf_id).get_reference("P"), Some(parent_id));
    assert_eq!(
        entries(&reopened, &root, "ParentTree", "Nums"),
        vec![
            PdfObject::Integer(7),
            reference(leaf_id),
            PdfObject::Integer(9),
            PdfObject::Array(vec![PdfObject::Null, reference(leaf_id)])
        ]
    );
    assert_eq!(
        reopened
            .reader()
            .get_object(nums_ref.0, nums_ref.1)
            .unwrap(),
        nums
    );
    assert_eq!(report.materialized_structure_nodes, 3);
    assert_eq!(report.repaired_structure_parents, 2);
    crate::tagged_structure::validate_parent_tree(&output).unwrap();
}

#[test]
fn stale_indirect_lookup_metadata_is_rejected_before_publication() {
    let f = fixture(false, false, true, false);
    let input = direct_owner_input(&f, false, false);
    let document = PdfDocument::open_bytes(input).unwrap();
    let mut root = dict(&document, f.structure);
    let lookup = root.get("ParentTree").unwrap().clone();
    let lookup_ref = (
        document
            .reader()
            .object_ids()
            .iter()
            .map(|r| r.0)
            .max()
            .unwrap()
            + 1,
        0,
    );
    root.insert("ParentTree", reference(lookup_ref));
    let input = write_objects(
        document.reader(),
        BTreeMap::from([
            (f.structure, PdfObject::Dictionary(root)),
            (lookup_ref, lookup.clone()),
        ]),
    );
    let current = PdfDocument::open_bytes(input.clone()).unwrap();
    let mut changed = lookup.as_dict().unwrap().clone();
    changed.insert("WFKeep", PdfObject::String(b"changed".to_vec()));
    let staged = write_objects(
        current.reader(),
        BTreeMap::from([(lookup_ref, PdfObject::Dictionary(changed))]),
    );
    assert!(stage(
        &input,
        &staged,
        &BTreeSet::from(["owned-direct".into()]),
        100_000
    )
    .is_err());
}

#[test]
fn competing_indirect_owner_shadows_are_not_silently_merged() {
    let f = fixture(false, false, true, false);
    let input = direct_owner_input(&f, false, true);
    let document = PdfDocument::open_bytes(input).unwrap();
    let mut root = dict(&document, f.structure);
    let second = (
        document
            .reader()
            .object_ids()
            .iter()
            .map(|r| r.0)
            .max()
            .unwrap()
            + 1,
        0,
    );
    let mut lookup = root.get_dict("IDTree").unwrap().clone();
    lookup.insert(
        "Names",
        PdfObject::Array(vec![
            PdfObject::String(b"annotation-owner".to_vec()),
            reference(second),
        ]),
    );
    root.insert("IDTree", PdfObject::Dictionary(lookup));
    let input = write_objects(
        document.reader(),
        BTreeMap::from([
            (f.structure, PdfObject::Dictionary(root)),
            (second, PdfObject::Dictionary(dict(&document, f.tag))),
        ]),
    );
    assert!(promote(&input).is_err());
}

#[test]
fn structure_dependency_closure_reaches_direct_field_siblings_on_another_page() {
    let f = fixture(false, false, true, false);
    let document = PdfDocument::open_bytes(f.bytes.clone()).unwrap();
    let p2 = document.get_page(2).unwrap();
    let page2 = (p2.object_number, p2.generation_number);
    let mut first = dict(&document, f.tag);
    first.remove("P");
    let mut widget = dict(&document, f.shadow);
    widget.insert("Subtype", PdfObject::Name("Widget".into()));
    widget.insert("P", reference(page2));
    widget.insert("NM", annotation_identity::text_string("dependent-widget"));
    widget.insert("StructParent", PdfObject::Integer(8));
    let mut sibling = widget.clone();
    sibling.remove("StructParent");
    sibling.insert("NM", annotation_identity::text_string("field-sibling"));
    let mut field = PdfDictionary::empty();
    field.insert("T", annotation_identity::text_string("group"));
    field.insert("FT", PdfObject::Name("Tx".into()));
    field.insert(
        "Kids",
        PdfObject::Array(vec![
            PdfObject::Dictionary(widget.clone()),
            PdfObject::Dictionary(sibling.clone()),
        ]),
    );
    let mut form = PdfDictionary::empty();
    form.insert(
        "Fields",
        PdfObject::Array(vec![PdfObject::Dictionary(field)]),
    );
    let mut objr = PdfDictionary::empty();
    objr.insert("Type", PdfObject::Name("OBJR".into()));
    objr.insert("Pg", reference(page2));
    objr.insert("Obj", PdfObject::Dictionary(widget.clone()));
    let mut second = PdfDictionary::empty();
    second.insert("Type", PdfObject::Name("StructElem".into()));
    second.insert("S", PdfObject::Name("Form".into()));
    second.insert("Pg", reference(page2));
    second.insert("K", PdfObject::Dictionary(objr));
    let mut lookup = PdfDictionary::empty();
    lookup.insert(
        "Nums",
        PdfObject::Array(vec![
            PdfObject::Integer(7),
            PdfObject::Dictionary(first.clone()),
            PdfObject::Integer(8),
            PdfObject::Dictionary(second.clone()),
        ]),
    );
    let mut root = dict(&document, f.structure);
    root.insert(
        "K",
        PdfObject::Array(vec![
            PdfObject::Dictionary(first),
            PdfObject::Dictionary(second),
        ]),
    );
    root.insert("ParentTree", PdfObject::Dictionary(lookup));
    root.insert("ParentTreeNextKey", PdfObject::Integer(9));
    let mut catalog = document.get_catalog().unwrap();
    catalog.insert("StructTreeRoot", PdfObject::Dictionary(root));
    catalog.insert("AcroForm", PdfObject::Dictionary(form));
    let mut page = dict(&document, page2);
    page.insert(
        "Annots",
        PdfObject::Array(vec![
            PdfObject::Dictionary(widget),
            PdfObject::Dictionary(sibling),
        ]),
    );
    let input = write_objects(
        document.reader(),
        BTreeMap::from([
            (f.catalog, PdfObject::Dictionary(catalog)),
            (page2, PdfObject::Dictionary(page)),
        ]),
    );
    let (output, report) = promote(&input).unwrap();
    let reopened = PdfDocument::open_bytes(output.clone()).unwrap();
    let ids = annotation_identity::index(&reopened, 100_000).unwrap();
    assert_eq!(report.changed_pages, vec![1, 2]);
    assert_eq!(report.promoted_ids.len(), 3);
    assert!(report.field_ownership_verified && report.tagged_ownership_verified);
    let parent = fields(&reopened)[0].as_reference().unwrap();
    for name in ["dependent-widget", "field-sibling"] {
        let r = ids
            .values()
            .find(|i| i.id == name)
            .unwrap()
            .reference
            .unwrap();
        assert_eq!(dict(&reopened, r).get_reference("Parent"), Some(parent));
        assert_eq!(dict(&reopened, r).get_reference("P"), Some(page2));
    }
    assert_eq!(report.materialized_field_nodes, 1);
    assert_eq!(report.materialized_structure_nodes, 3);
    crate::tagged_structure::validate_parent_tree(&output).unwrap();
}

#[test]
fn semantic_refs_reuse_declared_owner_and_indirect_names_normalize_without_guessing() {
    let f = fixture(false, false, true, false);
    let input = direct_owner_input(&f, false, false);
    let document = PdfDocument::open_bytes(input).unwrap();
    let next = document
        .reader()
        .object_ids()
        .iter()
        .map(|r| r.0)
        .max()
        .unwrap()
        + 1;
    let type_id = (next, 0);
    let role_id = (next + 1, 0);
    let mut owner = dict(&document, f.tag);
    owner.insert("Ref", PdfObject::Array(vec![reference(f.tag)]));
    owner.insert("S", reference(role_id));
    let mut root = dict(&document, f.structure);
    root.insert("Type", reference(type_id));
    root.insert("K", PdfObject::Dictionary(owner.clone()));
    let mut parents = root.get_dict("ParentTree").unwrap().clone();
    parents.insert(
        "Nums",
        PdfObject::Array(vec![
            PdfObject::Integer(7),
            PdfObject::Dictionary(owner.clone()),
        ]),
    );
    root.insert("ParentTree", PdfObject::Dictionary(parents));
    let mut names = root.get_dict("IDTree").unwrap().clone();
    names.insert(
        "Names",
        PdfObject::Array(vec![
            PdfObject::String(b"annotation-owner".to_vec()),
            PdfObject::Dictionary(owner.clone()),
        ]),
    );
    root.insert("IDTree", PdfObject::Dictionary(names));
    let input = write_objects(
        document.reader(),
        BTreeMap::from([
            (f.structure, PdfObject::Dictionary(root)),
            (f.tag, PdfObject::Dictionary(owner)),
            (type_id, PdfObject::Name("StructTreeRoot".into())),
            (role_id, PdfObject::Name("Annot".into())),
        ]),
    );
    let (output, _) = promote(&input).unwrap();
    assert_eq!(assert_owner(&output, &f), (f.structure, f.tag));
    let reopened = PdfDocument::open_bytes(output).unwrap();
    assert_eq!(
        dict(&reopened, f.structure).get_name("Type"),
        Some("StructTreeRoot")
    );
    assert_eq!(dict(&reopened, f.tag).get_name("S"), Some("Annot"));
    assert_eq!(
        dict(&reopened, f.tag).get_array("Ref"),
        Some([reference(f.tag)].as_slice())
    );
    let staged = write_objects(
        PdfDocument::open_bytes(input.clone()).unwrap().reader(),
        BTreeMap::from([(role_id, PdfObject::Name("P".into()))]),
    );
    assert!(stage(
        &input,
        &staged,
        &BTreeSet::from(["owned-direct".into()]),
        100_000
    )
    .is_err());
}

#[test]
fn contradictory_parent_and_null_semantic_refs_remain_explicit_errors() {
    let f = fixture(false, false, true, false);
    for broken_parent in [false, true] {
        let document = PdfDocument::open_bytes(f.bytes.clone()).unwrap();
        let mut owner = dict(&document, f.tag);
        if broken_parent {
            owner.insert("P", reference(f.catalog));
        } else {
            owner.insert("Ref", PdfObject::Array(vec![PdfObject::Null]));
        }
        let mut root = dict(&document, f.structure);
        root.insert("K", PdfObject::Dictionary(owner.clone()));
        let mut parents = root.get_dict("ParentTree").unwrap().clone();
        parents.insert(
            "Nums",
            PdfObject::Array(vec![PdfObject::Integer(7), PdfObject::Dictionary(owner)]),
        );
        root.insert("ParentTree", PdfObject::Dictionary(parents));
        let input = write_objects(
            document.reader(),
            BTreeMap::from([(f.structure, PdfObject::Dictionary(root))]),
        );
        assert!(promote(&input).is_err());
    }
}

#[test]
fn indirect_structure_owners_close_all_direct_objr_dependencies() {
    let f = fixture(false, false, true, false);
    let document = PdfDocument::open_bytes(f.bytes.clone()).unwrap();
    let p2 = document.get_page(2).unwrap();
    let page2 = (p2.object_number, p2.generation_number);
    let second_owner = (
        document
            .reader()
            .object_ids()
            .iter()
            .map(|r| r.0)
            .max()
            .unwrap()
            + 1,
        0,
    );
    let mut second = dict(&document, f.shadow);
    second.insert("NM", annotation_identity::text_string("another-direct"));
    second.insert("P", reference(page2));
    second.insert("StructParent", PdfObject::Integer(8));
    let mut owner = dict(&document, f.tag);
    owner.insert("Pg", reference(page2));
    let mut objr = owner.get_dict("K").unwrap().clone();
    objr.insert("Obj", PdfObject::Dictionary(second.clone()));
    objr.insert("Pg", reference(page2));
    owner.insert("K", PdfObject::Dictionary(objr));
    let mut page = dict(&document, page2);
    page.insert(
        "Annots",
        PdfObject::Array(vec![PdfObject::Dictionary(second)]),
    );
    let mut root = dict(&document, f.structure);
    root.insert(
        "K",
        PdfObject::Array(vec![reference(f.tag), reference(second_owner)]),
    );
    let mut lookup = root.get_dict("ParentTree").unwrap().clone();
    lookup.insert(
        "Nums",
        PdfObject::Array(vec![
            PdfObject::Integer(7),
            reference(f.tag),
            PdfObject::Integer(8),
            reference(second_owner),
        ]),
    );
    root.insert("ParentTree", PdfObject::Dictionary(lookup));
    root.insert("ParentTreeNextKey", PdfObject::Integer(9));
    let input = write_objects(
        document.reader(),
        BTreeMap::from([
            (f.structure, PdfObject::Dictionary(root)),
            (second_owner, PdfObject::Dictionary(owner)),
            (page2, PdfObject::Dictionary(page)),
        ]),
    );
    let (output, report) = promote(&input).unwrap();
    assert_eq!(report.materialized_structure_nodes, 0);
    assert_eq!(report.changed_pages, vec![1, 2]);
    assert_eq!(
        report.promoted_ids.into_iter().collect::<BTreeSet<_>>(),
        BTreeSet::from(["owned-direct".into(), "another-direct".into()])
    );
    let reopened = PdfDocument::open_bytes(output.clone()).unwrap();
    let ids = annotation_identity::index(&reopened, 100_000).unwrap();
    let second = ids
        .values()
        .find(|i| i.id == "another-direct")
        .unwrap()
        .reference
        .unwrap();
    assert_eq!(
        dict(&reopened, second_owner)
            .get_dict("K")
            .unwrap()
            .get_reference("Obj"),
        Some(second)
    );
    assert_eq!(dict(&reopened, second).get_reference("P"), Some(page2));
    crate::tagged_structure::validate_parent_tree(&output).unwrap();
}
