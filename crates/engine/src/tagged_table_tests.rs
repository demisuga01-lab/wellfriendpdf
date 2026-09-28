//! Source regression cases. Do not mistake their presence for execution evidence.
use super::*;
use crate::linked_stories::{apply_linked_story, load_linked_stories, preview_linked_story};

fn fixture(external_reference: bool) -> Vec<u8> {
    let bytes = crate::linked_stories::tables::tests::fixture(false);
    let engine = ContentEngine::open_bytes(bytes).unwrap();
    let mut store = Store::new(engine.document().reader());
    let root = store
        .add(PdfObject::Dictionary(PdfDictionary::empty()))
        .unwrap();
    let table = store
        .add(PdfObject::Dictionary(PdfDictionary::empty()))
        .unwrap();
    let row = store
        .add(PdfObject::Dictionary(PdfDictionary::empty()))
        .unwrap();
    let page = engine.document().get_page(1).unwrap();
    let page_id = (page.object_number, page.generation_number);
    let mut raw = b"/TH << /MCID 0 >> BDC\n".to_vec();
    for id in page.contents {
        let decoded = decode_stream_lossless_with_limits(
            &store.get(id).unwrap(),
            store.reader,
            &DecodeLimits::default(),
        )
        .unwrap();
        raw.extend(decoded.data);
        raw.push(b'\n');
    }
    raw.extend_from_slice(b"\nEMC\n");
    let mut d = PdfDictionary::empty();
    d.insert("Length", PdfObject::Integer(raw.len() as i64));
    let stream = store.add(PdfObject::Stream { dict: d, raw }).unwrap();
    let mut d = store.dict(page_id).unwrap();
    d.insert("Contents", reference(stream));
    store.replace_dict(page_id, d).unwrap();
    let mut d = PdfDictionary::empty();
    d.insert("Type", PdfObject::Name("StructElem".into()));
    d.insert("S", PdfObject::Name("TH".into()));
    d.insert("P", reference(row));
    d.insert("Pg", reference(page_id));
    d.insert("K", PdfObject::Integer(0));
    d.insert("ID", PdfObject::String(b"retained-header".to_vec()));
    let header = store.add(PdfObject::Dictionary(d)).unwrap();
    for (id, role, parent, child) in [(row, "TR", table, header), (table, "Table", root, row)] {
        let mut d = PdfDictionary::empty();
        d.insert("Type", PdfObject::Name("StructElem".into()));
        d.insert("S", PdfObject::Name(role.into()));
        d.insert("P", reference(parent));
        d.insert("K", reference(child));
        store.replace_dict(id, d).unwrap();
    }
    let mut d = PdfDictionary::empty();
    d.insert("Type", PdfObject::Name("StructElem".into()));
    d.insert("S", PdfObject::Name("H1".into()));
    d.insert("P", reference(root));
    d.insert("K", PdfObject::Array(Vec::new()));
    d.insert(
        "UnrelatedMetadata",
        PdfObject::String(b"preserve me".to_vec()),
    );
    if external_reference {
        d.insert("Ref", PdfObject::Array(vec![reference(header)]));
    }
    let untouched = store.add(PdfObject::Dictionary(d)).unwrap();
    let mut d = PdfDictionary::empty();
    d.insert("Type", PdfObject::Name("StructTreeRoot".into()));
    d.insert(
        "K",
        PdfObject::Array(vec![reference(table), reference(untouched)]),
    );
    store.replace_dict(root, d).unwrap();
    let catalog = store.reader.root_reference().unwrap();
    let mut d = store.dict(catalog).unwrap();
    d.insert("StructTreeRoot", reference(root));
    store.replace_dict(catalog, d).unwrap();
    rebuild_parent_tree(&write_store(store).unwrap(), "en")
        .unwrap()
        .0
}
fn request(input: &[u8], long: bool) -> LinkedStoryRequest {
    let mut request = crate::linked_stories::tables::tests::request(input, long);
    let inventory = sources(input).unwrap();
    let find = |role: &str| {
        inventory
            .iter()
            .find(|node| node.role == role)
            .unwrap()
            .reference
            .clone()
    };
    request.table_layout.as_mut().unwrap().tagging = Some(TableTagging {
        source: find("Table"),
        rows: BTreeMap::from([("header".into(), Some(find("TR"))), ("body".into(), None)]),
        cells: (0..4)
            .map(|i| {
                (
                    format!("c{i}"),
                    if i == 0 { Some(find("TH")) } else { None },
                )
            })
            .collect(),
        semantics: (0..4)
            .map(|i| {
                (
                    format!("c{i}"),
                    CellSemantics {
                        role: if i < 2 { CellRole::TH } else { CellRole::TD },
                        scope: (i < 2).then_some(HeaderScope::Column),
                        headers: if i >= 2 {
                            vec![format!("c{}", i - 2)]
                        } else {
                            Vec::new()
                        },
                    },
                )
            })
            .collect(),
        semantic_text: BTreeMap::new(),
        content_paths: BTreeMap::new(),
        blocks: BTreeMap::new(),
        groups: Vec::new(),
        removed_groups: Vec::new(),
        row_text: BTreeMap::new(),
        table_text: None,
    });
    request
}

#[test]
fn single_leaf_binding_does_not_silently_flatten_multiple_cell_blocks() {
    let input = fixture(false);
    let mut request = request(&input, false);
    request.table_layout.as_mut().unwrap().cells[0].paragraph_ids = vec!["c0".into()];
    validate_config(&request).unwrap();
    let mut additional = request.paragraphs[0].clone();
    additional.id = "additional-header-block".into();
    additional.text = "Another paragraph".into();
    request.paragraphs.push(additional);
    request.table_layout.as_mut().unwrap().cells[0]
        .paragraph_ids
        .push("additional-header-block".into());
    assert!(validate_config(&request).is_err());
    assert!(preview_linked_story(&input, &request).is_err());
    let tags = request
        .table_layout
        .as_mut()
        .unwrap()
        .tagging
        .as_mut()
        .unwrap();
    tags.blocks.insert("c0".into(), new_block());
    tags.blocks
        .insert("additional-header-block".into(), new_block());
    assert!(preview_linked_story(&input, &request).is_ok());
}

#[test]
fn blank_cell_line_carriers_keep_mcid_ownership_through_save_and_reopen() {
    let input = fixture(false);
    let mut edit = request(&input, false);
    edit.paragraphs[2].text = "\nB\r\n\u{2028}".into();
    let (output, _) = apply_linked_story(&input, &edit).unwrap();
    validate_parent_tree(&output).unwrap();
    let engine = ContentEngine::open_bytes(output.clone()).unwrap();
    let mut carried = String::new();
    for page in 1..=engine.page_count().unwrap() {
        let resources = engine.get_page_resources(page).unwrap();
        for item in engine.collect_page_scoped_text_chunks(page).unwrap() {
            if resources
                .fonts
                .get(&item.chunk.font_name)
                .is_some_and(crate::advanced_editing::story_carriers::is_font)
            {
                assert!(item.mcid.is_some());
                assert!(item.chunk.is_actual_text);
                assert_eq!(item.chunk.width, 0.0);
                carried.push_str(&item.chunk.text);
            }
        }
    }
    assert_eq!(carried, "\n\u{2028}");
    let saved = load_linked_stories(&output).unwrap().remove(0).request;
    let (repeated, _) = apply_linked_story(&output, &saved).unwrap();
    validate_parent_tree(&repeated).unwrap();
    assert_eq!(
        load_linked_stories(&repeated).unwrap()[0]
            .request
            .paragraphs[2]
            .text,
        edit.paragraphs[2].text
    );
}

fn new_block() -> CellBlockTagging {
    CellBlockTagging {
        path: Vec::new(),
        new_role: None,
        semantic_text: None,
    }
}
#[test]
fn block_cells_keep_header_references_through_growth_reorder_delete_and_reopen() {
    let input = fixture(true);
    let mut request = request(&input, true);
    let mut extra = request.paragraphs[0].clone();
    extra.id = "header-detail".into();
    extra.text = "Units".into();
    extra.font_size = 8.0;
    let mut empty = request.paragraphs[2].clone();
    empty.id = "body-empty".into();
    empty.text.clear();
    request.paragraphs.extend([extra, empty]);
    let table = request.table_layout.as_mut().unwrap();
    table.cells[0].paragraph_ids = vec!["c0".into(), "header-detail".into()];
    table.cells[2].paragraph_ids = vec!["c2".into(), "body-empty".into()];
    let tags = table.tagging.as_mut().unwrap();
    for id in ["c0", "header-detail", "c2", "body-empty"] {
        tags.blocks.insert(id.into(), new_block());
    }
    let (output, preview) = apply_linked_story(&input, &request).unwrap();
    assert!(preview.generated_pages > 1);
    validate_parent_tree(&output).unwrap();
    let mut saved = load_linked_stories(&output).unwrap().remove(0).request;
    let tags = saved
        .table_layout
        .as_ref()
        .unwrap()
        .tagging
        .as_ref()
        .unwrap();
    assert_eq!(tags.blocks.len(), 4);
    let cell = tags.cells["c0"].as_ref().unwrap();
    let leaf = &tags.blocks["c0"].path[0].source;
    assert_ne!(cell.key, leaf.key);
    let engine = ContentEngine::open_bytes(output.clone()).unwrap();
    let (store, _, _) = index_document(&engine, None).unwrap();
    let cell_id = (cell.object, cell.generation);
    let d = store.dict(cell_id).unwrap();
    assert_eq!(d.get_name("S"), Some("TH"));
    assert!(!d.contains_key("ActualText"));
    assert_eq!(
        d.get("ID").unwrap().as_string().unwrap(),
        b"retained-header"
    );
    assert_eq!(kids(&d).len(), 2);
    let leaf_dict = store.dict((leaf.object, leaf.generation)).unwrap();
    assert_eq!(leaf_dict.get_name("S"), Some("P"));
    assert_eq!(leaf_dict.get_reference("P"), Some(cell_id));
    let inventory = sources(&output).unwrap();
    let unrelated = inventory.iter().find(|n| n.role == "H1").unwrap();
    let d = store
        .dict((unrelated.reference.object, unrelated.reference.generation))
        .unwrap();
    assert_eq!(
        d.get("Ref").unwrap().as_array().unwrap()[0].as_reference(),
        Some(cell_id)
    );
    assert_eq!(
        inventory
            .iter()
            .map(|node| node.page_mcids.len())
            .sum::<usize>(),
        preview
            .frames
            .iter()
            .flat_map(|f| &f.lines)
            .filter(|line| !line.artifact)
            .count()
    );
    let blank = &tags.blocks["body-empty"].path[0].source;
    assert!(kids(&store.dict((blank.object, blank.generation)).unwrap()).is_empty());
    saved.table_layout.as_mut().unwrap().cells[0]
        .paragraph_ids
        .reverse();
    let (reordered, _) = apply_linked_story(&output, &saved).unwrap();
    validate_parent_tree(&reordered).unwrap();
    let mut saved = load_linked_stories(&reordered).unwrap().remove(0).request;
    let tags = saved
        .table_layout
        .as_ref()
        .unwrap()
        .tagging
        .as_ref()
        .unwrap();
    let first = tags.blocks["header-detail"].path[0].source.clone();
    let cell = tags.cells["c0"].as_ref().unwrap();
    let engine = ContentEngine::open_bytes(reordered.clone()).unwrap();
    let (store, _, _) = index_document(&engine, None).unwrap();
    assert_eq!(
        kids(&store.dict((cell.object, cell.generation)).unwrap())[0].as_reference(),
        Some((first.object, first.generation))
    );
    let removed_key = first.key.unwrap();
    saved.paragraphs.retain(|p| p.id != "header-detail");
    let table = saved.table_layout.as_mut().unwrap();
    table.cells[0].paragraph_ids = vec!["c0".into()];
    table
        .tagging
        .as_mut()
        .unwrap()
        .blocks
        .remove("header-detail");
    let (deleted, _) = apply_linked_story(&reordered, &saved).unwrap();
    validate_parent_tree(&deleted).unwrap();
    assert!(sources(&deleted)
        .unwrap()
        .iter()
        .all(|s| s.reference.key.as_ref() != Some(&removed_key)));
}

fn shared_block_fixture(external_leaf_reference: bool) -> Vec<u8> {
    let input = nested_group_fixture();
    let inventory = sources(&input).unwrap();
    let find = |role: &str| {
        let s = inventory.iter().find(|s| s.role == role).unwrap();
        (s.reference.object, s.reference.generation)
    };
    let engine = ContentEngine::open_bytes(input.clone()).unwrap();
    let (mut store, _, _) = index_document(&engine, None).unwrap();
    let parent = find("P");
    let mut d = PdfDictionary::empty();
    d.insert("Type", PdfObject::Name("StructElem".into()));
    d.insert("S", PdfObject::Name("Span".into()));
    d.insert("P", reference(parent));
    d.insert("K", PdfObject::Array(Vec::new()));
    d.insert("ID", PdfObject::String(b"second-block".to_vec()));
    let second = store.add(PdfObject::Dictionary(d)).unwrap();
    let mut d = store.dict(parent).unwrap();
    let mut children = kids(&d);
    children.push(reference(second));
    d.insert("K", PdfObject::Array(children));
    store.replace_dict(parent, d).unwrap();
    if external_leaf_reference {
        let outside = find("H1");
        let mut d = store.dict(outside).unwrap();
        d.insert("Ref", PdfObject::Array(vec![reference(second)]));
        store.replace_dict(outside, d).unwrap();
    }
    rebuild_owner_trees(&write_store(store).unwrap(), None)
        .unwrap()
        .0
}
fn shared_block_request(input: &[u8]) -> LinkedStoryRequest {
    let mut request = nested_request(input);
    let inventory = sources(input).unwrap();
    let first = inventory
        .iter()
        .find(|s| s.role == "Span" && !s.page_mcids.is_empty())
        .unwrap()
        .reference
        .clone();
    let second = inventory
        .iter()
        .find(|s| s.role == "Span" && s.page_mcids.is_empty())
        .unwrap()
        .reference
        .clone();
    let mut extra = request.paragraphs[0].clone();
    extra.id = "header-detail".into();
    extra.text = "Units".into();
    request.paragraphs.push(extra);
    let table = request.table_layout.as_mut().unwrap();
    table.cells[0].paragraph_ids = vec!["c0".into(), "header-detail".into()];
    let tags = table.tagging.as_mut().unwrap();
    let mut path = tags.content_paths.remove("c0").unwrap();
    path.last_mut().unwrap().source = first;
    tags.blocks.insert(
        "c0".into(),
        CellBlockTagging {
            path: path.clone(),
            new_role: None,
            semantic_text: None,
        },
    );
    path.last_mut().unwrap().source = second;
    path.last_mut().unwrap().semantic_text = None;
    tags.blocks.insert(
        "header-detail".into(),
        CellBlockTagging {
            path,
            new_role: None,
            semantic_text: None,
        },
    );
    request
}
#[test]
fn shared_block_ancestors_and_descriptions_survive_repeated_editing() {
    let input = shared_block_fixture(false);
    let request = shared_block_request(&input);
    let (output, _) = apply_linked_story(&input, &request).unwrap();
    validate_parent_tree(&output).unwrap();
    let mut saved = load_linked_stories(&output).unwrap().remove(0).request;
    let tags = saved
        .table_layout
        .as_ref()
        .unwrap()
        .tagging
        .as_ref()
        .unwrap();
    let a = &tags.blocks["c0"].path;
    let b = &tags.blocks["header-detail"].path;
    assert_eq!(a.len(), 2);
    assert_eq!(b.len(), 2);
    assert_eq!(a[0].source, b[0].source);
    let engine = ContentEngine::open_bytes(output.clone()).unwrap();
    let (store, _, _) = index_document(&engine, None).unwrap();
    let d = store
        .dict((a[0].source.object, a[0].source.generation))
        .unwrap();
    assert_eq!(
        d.get("ID").unwrap().as_string().unwrap(),
        b"nested-block-id"
    );
    assert_eq!(kids(&d).len(), 2);
    assert!(!d.contains_key("ActualText"));
    assert!(preview_linked_story(&output, &saved).is_err());
    for block in saved
        .table_layout
        .as_mut()
        .unwrap()
        .tagging
        .as_mut()
        .unwrap()
        .blocks
        .values_mut()
    {
        for node in &mut block.path {
            node.semantic_text = Some(StorySemanticText {
                alternate: None,
                expansion: None,
            });
        }
    }
    saved.table_layout.as_mut().unwrap().cells[0]
        .paragraph_ids
        .reverse();
    let (again, _) = apply_linked_story(&output, &saved).unwrap();
    validate_parent_tree(&again).unwrap();
    let loaded = load_linked_stories(&again).unwrap();
    assert_eq!(
        loaded[0].request.table_layout.as_ref().unwrap().cells[0].paragraph_ids,
        vec!["header-detail", "c0"]
    );
}
#[test]
fn conflicting_shared_reviews_and_referenced_block_removal_fail_closed() {
    let input = shared_block_fixture(true);
    let mut request = shared_block_request(&input);
    request
        .table_layout
        .as_mut()
        .unwrap()
        .tagging
        .as_mut()
        .unwrap()
        .blocks
        .get_mut("header-detail")
        .unwrap()
        .path[0]
        .semantic_text = Some(StorySemanticText {
        alternate: Some("conflicting shared description".into()),
        expansion: None,
    });
    assert!(preview_linked_story(&input, &request).is_err());
    let mut request = shared_block_request(&input);
    request.paragraphs.retain(|p| p.id != "header-detail");
    let table = request.table_layout.as_mut().unwrap();
    table.cells[0].paragraph_ids = vec!["c0".into()];
    table
        .tagging
        .as_mut()
        .unwrap()
        .blocks
        .remove("header-detail");
    assert!(preview_linked_story(&input, &request).is_err());
}

#[test]
fn tagged_table_growth_headers_ids_parent_tree_and_repeat_edit() {
    let input = fixture(false);
    let request = request(&input, true);
    let (output, preview) = apply_linked_story(&input, &request).unwrap();
    assert!(preview.generated_pages > 1);
    validate_parent_tree(&output).unwrap();
    assert!(preview
        .frames
        .iter()
        .flat_map(|f| &f.lines)
        .filter(|line| line.artifact)
        .all(|line| line.tag_owner.is_none()));
    let inventory = sources(&output).unwrap();
    let physical = inventory
        .iter()
        .map(|node| node.page_mcids.len())
        .sum::<usize>();
    let logical = preview
        .frames
        .iter()
        .flat_map(|f| &f.lines)
        .filter(|line| !line.artifact)
        .count();
    assert_eq!(physical, logical);
    let engine = ContentEngine::open_bytes(output.clone()).unwrap();
    let (store, _, _) = index_document(&engine, None).unwrap();
    let header = inventory
        .iter()
        .find(|node| node.reference.key.as_deref() == Some(key(&request.story_id, "c0").as_str()))
        .unwrap();
    let d = store
        .dict((header.reference.object, header.reference.generation))
        .unwrap();
    assert_eq!(
        d.get("ID").unwrap().as_string().unwrap(),
        b"retained-header"
    );
    let body = inventory
        .iter()
        .find(|node| node.reference.key.as_deref() == Some(key(&request.story_id, "c2").as_str()))
        .unwrap();
    let d = store
        .dict((body.reference.object, body.reference.generation))
        .unwrap();
    let attributes = store.resolve(d.get("A").unwrap()).unwrap();
    let attributes = match &attributes {
        PdfObject::Array(a) => a
            .iter()
            .filter_map(PdfObject::as_dict)
            .find(|d| d.get_name("O") == Some("Table"))
            .unwrap(),
        PdfObject::Dictionary(d) => d,
        _ => panic!("invalid attributes"),
    };
    assert_eq!(attributes.get_integer("RowSpan"), Some(1));
    assert_eq!(
        attributes.get("Headers").unwrap().as_array().unwrap()[0]
            .as_string()
            .unwrap(),
        b"retained-header"
    );
    let untouched = inventory.iter().find(|node| node.role == "H1").unwrap();
    assert_eq!(
        store
            .dict((untouched.reference.object, untouched.reference.generation))
            .unwrap()
            .get("UnrelatedMetadata")
            .unwrap()
            .as_string()
            .unwrap(),
        b"preserve me"
    );
    let mut saved = load_linked_stories(&output).unwrap().remove(0).request;
    let tags = saved
        .table_layout
        .as_ref()
        .unwrap()
        .tagging
        .as_ref()
        .unwrap();
    assert!(
        tags.source.key.is_some()
            && tags
                .rows
                .values()
                .all(|v| v.as_ref().unwrap().key.is_some())
    );
    saved.paragraphs[2].text = "Short replacement".into();
    let (short, _) = apply_linked_story(&output, &saved).unwrap();
    validate_parent_tree(&short).unwrap();
    let mut saved = load_linked_stories(&short).unwrap().remove(0).request;
    saved.paragraphs[2].text = "Another replacement".into();
    let (again, _) = apply_linked_story(&short, &saved).unwrap();
    validate_parent_tree(&again).unwrap();
}
#[test]
fn header_cycles_and_external_references_are_not_discarded() {
    let input = fixture(true);
    let mut request = request(&input, false);
    request
        .table_layout
        .as_mut()
        .unwrap()
        .tagging
        .as_mut()
        .unwrap()
        .cells
        .insert("c0".into(), None);
    assert!(preview_linked_story(&input, &request).is_err());
    let mut request = self::request(&input, false);
    let tags = request
        .table_layout
        .as_mut()
        .unwrap()
        .tagging
        .as_mut()
        .unwrap();
    tags.semantics.get_mut("c0").unwrap().headers = vec!["c1".into()];
    tags.semantics.get_mut("c1").unwrap().headers = vec!["c0".into()];
    assert!(preview_linked_story(&input, &request).is_err());
}
#[test]
fn changing_spans_updates_attributes_without_changing_cell_id() {
    let input = fixture(false);
    let (output, _) = apply_linked_story(&input, &request(&input, false)).unwrap();
    let mut saved = load_linked_stories(&output).unwrap().remove(0).request;
    let mut p = saved.paragraphs[3].clone();
    p.id = "c4".into();
    p.text = "Second row".into();
    saved.paragraphs.push(p);
    let table = saved.table_layout.as_mut().unwrap();
    table.rows.push(crate::linked_stories::tables::TableRow {
        id: "body2".into(),
        min_height: 90.0,
        allow_split: true,
        keep_with_next: false,
        break_before: false,
    });
    table.rows[1].min_height = 90.0;
    table.cells[2].row_span = 2;
    let mut cell = table.cells[3].clone();
    cell.id = "c4".into();
    cell.row = 2;
    table.cells.push(cell);
    let tags = table.tagging.as_mut().unwrap();
    tags.rows.insert("body2".into(), None);
    tags.cells.insert("c4".into(), None);
    tags.semantics.insert(
        "c4".into(),
        CellSemantics {
            role: CellRole::TD,
            scope: None,
            headers: vec!["c1".into()],
        },
    );
    let (output, preview) = apply_linked_story(&output, &saved).unwrap();
    assert!(preview.generated_pages > 0);
    validate_parent_tree(&output).unwrap();
    let inventory = sources(&output).unwrap();
    let cell = inventory
        .iter()
        .find(|node| node.reference.key.as_deref() == Some(key(&saved.story_id, "c2").as_str()))
        .unwrap();
    let engine = ContentEngine::open_bytes(output).unwrap();
    let (store, _, _) = index_document(&engine, None).unwrap();
    let d = store
        .dict((cell.reference.object, cell.reference.generation))
        .unwrap();
    let a = store.resolve(d.get("A").unwrap()).unwrap();
    let attrs = a
        .as_array()
        .unwrap()
        .iter()
        .filter_map(PdfObject::as_dict)
        .find(|a| a.get_name("O") == Some("Table"))
        .unwrap();
    assert_eq!(attrs.get_integer("RowSpan"), Some(2));
}

fn nested_group_fixture() -> Vec<u8> {
    let input = fixture(false);
    let engine = ContentEngine::open_bytes(input).unwrap();
    let (mut store, root, index) = index_document(&engine, None).unwrap();
    let find = |role: &str| {
        *index
            .nodes
            .iter()
            .find(|id| store.dict(**id).unwrap().get_name("S") == Some(role))
            .unwrap()
    };
    let table = find("Table");
    let row = find("TR");
    let cell = find("TH");
    let unrelated = find("H1");
    let block = store
        .add(PdfObject::Dictionary(PdfDictionary::empty()))
        .unwrap();
    let mut cell_dict = store.dict(cell).unwrap();
    let mut leaf = PdfDictionary::empty();
    leaf.insert("Type", PdfObject::Name("StructElem".into()));
    leaf.insert("S", PdfObject::Name("Span".into()));
    leaf.insert("P", reference(block));
    leaf.insert("Pg", cell_dict.get("Pg").unwrap().clone());
    leaf.insert("K", cell_dict.get("K").unwrap().clone());
    leaf.insert("Alt", logical_string("Old nested wording"));
    let leaf = store.add(PdfObject::Dictionary(leaf)).unwrap();
    let mut d = PdfDictionary::empty();
    d.insert("Type", PdfObject::Name("StructElem".into()));
    d.insert("S", PdfObject::Name("P".into()));
    d.insert("P", reference(cell));
    d.insert("K", reference(leaf));
    d.insert("ID", PdfObject::String(b"nested-block-id".to_vec()));
    store.replace_dict(block, d).unwrap();
    cell_dict.remove("Pg");
    cell_dict.insert("K", reference(block));
    cell_dict.insert("ActualText", logical_string("stale cell-level wording"));
    cell_dict.insert(
        "C",
        PdfObject::Array(vec![
            PdfObject::Name("Shared".into()),
            PdfObject::Integer(3),
        ]),
    );
    cell_dict.insert("R", PdfObject::Integer(3));
    let mut local = PdfDictionary::empty();
    local.insert("O", PdfObject::Name("Layout".into()));
    local.insert(
        "Color",
        PdfObject::Array(vec![
            PdfObject::Integer(0),
            PdfObject::Integer(0),
            PdfObject::Integer(1),
        ]),
    );
    cell_dict.insert(
        "A",
        PdfObject::Array(vec![PdfObject::Dictionary(local), PdfObject::Integer(3)]),
    );
    store.replace_dict(cell, cell_dict).unwrap();
    let mut shared = PdfDictionary::empty();
    shared.insert("O", PdfObject::Name("Layout".into()));
    shared.insert("BBox", PdfObject::Array(vec![PdfObject::Integer(0); 4]));
    shared.insert(
        "Color",
        PdfObject::Array(vec![
            PdfObject::Integer(1),
            PdfObject::Integer(0),
            PdfObject::Integer(0),
        ]),
    );
    shared.insert("Padding", PdfObject::Integer(2));
    let shared = store.add(PdfObject::Dictionary(shared)).unwrap();
    let mut classes = PdfDictionary::empty();
    classes.insert("Shared", reference(shared));
    let mut d = store.dict(root).unwrap();
    d.insert("ClassMap", PdfObject::Dictionary(classes));
    store.replace_dict(root, d).unwrap();
    let mut d = store.dict(unrelated).unwrap();
    d.insert("C", PdfObject::Name("Shared".into()));
    d.insert("R", PdfObject::Integer(2));
    store.replace_dict(unrelated, d).unwrap();
    let mut groups = Vec::new();
    for (role, children) in [("THead", vec![reference(row)]), ("TBody", Vec::new())] {
        let mut d = PdfDictionary::empty();
        d.insert("Type", PdfObject::Name("StructElem".into()));
        d.insert("S", PdfObject::Name(role.into()));
        d.insert("P", reference(table));
        d.insert("K", PdfObject::Array(children));
        groups.push(store.add(PdfObject::Dictionary(d)).unwrap());
    }
    let mut d = store.dict(row).unwrap();
    d.insert("P", reference(groups[0]));
    store.replace_dict(row, d).unwrap();
    let mut d = store.dict(table).unwrap();
    d.insert(
        "K",
        PdfObject::Array(groups.into_iter().map(reference).collect()),
    );
    store.replace_dict(table, d).unwrap();
    rebuild_owner_trees(&write_store(store).unwrap(), None)
        .unwrap()
        .0
}
fn nested_request(input: &[u8]) -> LinkedStoryRequest {
    let mut request = request(input, true);
    let inventory = sources(input).unwrap();
    let find = |role: &str| {
        inventory
            .iter()
            .find(|node| node.role == role)
            .unwrap()
            .reference
            .clone()
    };
    let tags = request
        .table_layout
        .as_mut()
        .unwrap()
        .tagging
        .as_mut()
        .unwrap();
    tags.content_paths.insert(
        "c0".into(),
        vec![
            CellTextOwner {
                source: find("P"),
                semantic_text: None,
            },
            CellTextOwner {
                source: find("Span"),
                semantic_text: Some(StorySemanticText {
                    alternate: Some("Approved nested wording".into()),
                    expansion: None,
                }),
            },
        ],
    );
    tags.groups = vec![
        RowGroupBinding {
            id: "head".into(),
            role: RowGroupRole::THead,
            rows: vec!["header".into()],
            source: Some(find("THead")),
            semantic_text: None,
        },
        RowGroupBinding {
            id: "body-group".into(),
            role: RowGroupRole::TBody,
            rows: vec!["body".into()],
            source: Some(find("TBody")),
            semantic_text: None,
        },
    ];
    request
}
#[test]
fn nested_paths_groups_classes_and_semantic_review_survive_reopen() {
    let input = nested_group_fixture();
    let request = nested_request(&input);
    let (output, preview) = apply_linked_story(&input, &request).unwrap();
    assert!(preview.generated_pages > 1);
    validate_parent_tree(&output).unwrap();
    let inventory = sources(&output).unwrap();
    let engine = ContentEngine::open_bytes(output.clone()).unwrap();
    let (store, root, _) = index_document(&engine, None).unwrap();
    let mut saved = load_linked_stories(&output).unwrap().remove(0).request;
    let tags = saved
        .table_layout
        .as_ref()
        .unwrap()
        .tagging
        .as_ref()
        .unwrap();
    assert_eq!(tags.content_paths["c0"].len(), 2);
    assert_eq!(tags.groups.len(), 2);
    assert!(inventory
        .iter()
        .find(|node| node.role == "Table")
        .unwrap()
        .stable_keys
        .contains(tags.source.key.as_ref().unwrap()));
    let cell = tags.cells["c0"].as_ref().unwrap();
    let d = store.dict((cell.object, cell.generation)).unwrap();
    assert_eq!(d.get_name("S"), Some("TH"));
    assert!(!d.contains_key("ActualText"));
    assert!(!d.contains_key("C"));
    assert_eq!(d.get_integer("R"), Some(4));
    assert_eq!(
        d.get("ID").unwrap().as_string().unwrap(),
        b"retained-header"
    );
    let block = &tags.content_paths["c0"][0].source;
    let b = store.dict((block.object, block.generation)).unwrap();
    assert_eq!(b.get_name("S"), Some("P"));
    assert_eq!(
        b.get("ID").unwrap().as_string().unwrap(),
        b"nested-block-id"
    );
    assert_eq!(b.get_reference("P"), Some((cell.object, cell.generation)));
    let leaf = &tags.content_paths["c0"][1].source;
    let l = store.dict((leaf.object, leaf.generation)).unwrap();
    assert_eq!(l.get_name("S"), Some("Span"));
    assert_eq!(l.get_reference("P"), Some((block.object, block.generation)));
    assert_eq!(
        semantic_string(&store, &l, "ActualText")
            .unwrap()
            .as_deref(),
        Some("Name")
    );
    let resolver = attributes::Resolver::new(&store).unwrap();
    let mut budget = attributes::Budget::default();
    let effective = resolver.effective(&store, &d, &mut budget).unwrap();
    let layout = effective
        .iter()
        .filter_map(PdfObject::as_dict)
        .find(|d| d.get_name("O") == Some("Layout"))
        .unwrap();
    assert!(!layout.contains_key("BBox"));
    assert_eq!(layout.get_integer("Padding"), Some(2));
    assert_eq!(
        layout.get("Color"),
        Some(&PdfObject::Array(vec![
            PdfObject::Integer(0),
            PdfObject::Integer(0),
            PdfObject::Integer(1)
        ]))
    );
    let classes = store
        .resolve(store.dict(root).unwrap().get("ClassMap").unwrap())
        .unwrap();
    let shared = store
        .resolve(classes.as_dict().unwrap().get("Shared").unwrap())
        .unwrap();
    assert!(shared.as_dict().unwrap().contains_key("BBox"));
    let unrelated = inventory.iter().find(|n| n.role == "H1").unwrap();
    let untouched = store
        .dict((unrelated.reference.object, unrelated.reference.generation))
        .unwrap();
    assert_eq!(untouched.get_name("C"), Some("Shared"));
    assert_eq!(untouched.get_integer("R"), Some(2));
    saved.paragraphs[2].text = "Contraction".into();
    assert!(preview_linked_story(&output, &saved).is_err()); // retained leaf Alt needs renewed approval
    saved
        .table_layout
        .as_mut()
        .unwrap()
        .tagging
        .as_mut()
        .unwrap()
        .content_paths
        .get_mut("c0")
        .unwrap()[1]
        .semantic_text = Some(StorySemanticText {
        alternate: None,
        expansion: None,
    });
    let (short, _) = apply_linked_story(&output, &saved).unwrap();
    validate_parent_tree(&short).unwrap();
    let saved = load_linked_stories(&short).unwrap().remove(0).request;
    apply_linked_story(&short, &saved).unwrap();
}
#[test]
fn groups_and_nested_paths_cannot_be_implicitly_flattened() {
    let input = nested_group_fixture();
    let mut request = nested_request(&input);
    request
        .table_layout
        .as_mut()
        .unwrap()
        .tagging
        .as_mut()
        .unwrap()
        .content_paths
        .clear();
    assert!(preview_linked_story(&input, &request).is_err());
    let mut request = nested_request(&input);
    request
        .table_layout
        .as_mut()
        .unwrap()
        .tagging
        .as_mut()
        .unwrap()
        .groups
        .clear();
    assert!(preview_linked_story(&input, &request).is_err());
    let mut request = nested_request(&input);
    let tags = request
        .table_layout
        .as_mut()
        .unwrap()
        .tagging
        .as_mut()
        .unwrap();
    tags.removed_groups = tags
        .groups
        .iter()
        .filter_map(|g| g.source.clone())
        .collect();
    tags.groups.clear();
    let (output, _) = apply_linked_story(&input, &request).unwrap();
    validate_parent_tree(&output).unwrap();
    assert!(!sources(&output)
        .unwrap()
        .iter()
        .any(|node| matches!(node.role.as_str(), "THead" | "TBody" | "TFoot")));
}

#[test]
fn class_header_references_respect_later_direct_overrides() {
    for overridden in [false, true] {
        let input = fixture(false);
        let engine = ContentEngine::open_bytes(input).unwrap();
        let (mut store, root, index) = index_document(&engine, None).unwrap();
        let unrelated = *index
            .nodes
            .iter()
            .find(|id| store.dict(**id).unwrap().get_name("S") == Some("H1"))
            .unwrap();
        let mut class = PdfDictionary::empty();
        class.insert("O", PdfObject::Name("Table".into()));
        class.insert(
            "Headers",
            PdfObject::Array(vec![PdfObject::String(b"retained-header".to_vec())]),
        );
        let mut classes = PdfDictionary::empty();
        classes.insert("ReferencesOldHeader", PdfObject::Dictionary(class));
        let mut d = store.dict(root).unwrap();
        d.insert("ClassMap", PdfObject::Dictionary(classes));
        store.replace_dict(root, d).unwrap();
        let mut d = store.dict(unrelated).unwrap();
        d.insert("C", PdfObject::Name("ReferencesOldHeader".into()));
        if overridden {
            let mut a = PdfDictionary::empty();
            a.insert("O", PdfObject::Name("Table".into()));
            a.insert("Headers", PdfObject::Array(Vec::new()));
            d.insert("A", PdfObject::Dictionary(a));
        }
        store.replace_dict(unrelated, d).unwrap();
        let input = write_store(store).unwrap();
        let mut request = request(&input, false);
        request
            .table_layout
            .as_mut()
            .unwrap()
            .tagging
            .as_mut()
            .unwrap()
            .cells
            .insert("c0".into(), None);
        let result = preview_linked_story(&input, &request);
        assert_eq!(result.is_ok(), overridden);
    }
}

#[test]
fn caption_subtree_and_inherited_page_survive_table_page_insertion() {
    use crate::authoring::{FontFace, PageSize, PdfBuilder, TextStyle};
    let source = ContentEngine::open_bytes(fixture(false)).unwrap();
    let mut builder = PdfBuilder::new();
    builder
        .add_page(PageSize::custom(220.0, 160.0))
        .draw_text(
            "CAPTION",
            10.0,
            130.0,
            &TextStyle::new(
                FontFace::Standard(crate::authoring::StandardFont::Helvetica),
                10.0,
            ),
        )
        .unwrap();
    let added = ContentEngine::open_bytes(builder.to_bytes().unwrap()).unwrap();
    let bytes = crate::writer::insert_authored_pages_preserving_catalog(
        source.document(),
        &[(added.document(), None)],
        2,
    )
    .unwrap();
    let engine = ContentEngine::open_bytes(bytes).unwrap();
    let (mut store, _, index) = index_document(&engine, None).unwrap();
    let table = *index
        .nodes
        .iter()
        .find(|id| store.dict(**id).unwrap().get_name("S") == Some("Table"))
        .unwrap();
    let page = engine.document().get_page(2).unwrap();
    let page_id = (page.object_number, page.generation_number);
    let mut raw = b"/Caption << /MCID 0 >> BDC\n".to_vec();
    for stream in page.contents {
        raw.extend(
            decode_stream_lossless_with_limits(
                &store.get(stream).unwrap(),
                store.reader,
                &DecodeLimits::default(),
            )
            .unwrap()
            .data,
        );
        raw.push(b'\n');
    }
    raw.extend_from_slice(b"\nEMC\n");
    let mut d = PdfDictionary::empty();
    d.insert("Length", PdfObject::Integer(raw.len() as i64));
    let stream = store.add(PdfObject::Stream { dict: d, raw }).unwrap();
    let mut d = store.dict(page_id).unwrap();
    d.insert("Contents", reference(stream));
    store.replace_dict(page_id, d).unwrap();
    let mut d = PdfDictionary::empty();
    d.insert("Type", PdfObject::Name("StructElem".into()));
    d.insert("S", PdfObject::Name("Caption".into()));
    d.insert("P", reference(table));
    d.insert("K", PdfObject::Integer(0));
    d.insert("ID", PdfObject::String(b"caption-preserved".to_vec()));
    let caption = store.add(PdfObject::Dictionary(d)).unwrap();
    let mut d = store.dict(table).unwrap();
    let mut k = kids(&d);
    k.insert(0, reference(caption));
    d.insert("K", PdfObject::Array(k));
    d.insert("Pg", reference(page_id));
    store.replace_dict(table, d).unwrap();
    let input = rebuild_owner_trees(&write_store(store).unwrap(), None)
        .unwrap()
        .0;
    let (output, preview) = apply_linked_story(&input, &request(&input, true)).unwrap();
    assert!(preview.generated_pages > 0);
    validate_parent_tree(&output).unwrap();
    let engine = ContentEngine::open_bytes(output.clone()).unwrap();
    let count = engine.page_count().unwrap();
    assert!(engine.get_page_text(count).unwrap().contains("CAPTION"));
    let inventory = sources(&output).unwrap();
    let caption = inventory.iter().find(|n| n.role == "Caption").unwrap();
    assert_eq!(caption.page_mcids, vec![(count, 0)]);
    let (store, _, _) = index_document(&engine, None).unwrap();
    let d = store
        .dict((caption.reference.object, caption.reference.generation))
        .unwrap();
    assert!(d.get_reference("Pg").is_some());
    assert_eq!(
        d.get("ID").unwrap().as_string().unwrap(),
        b"caption-preserved"
    );
    let saved = load_linked_stories(&output).unwrap().remove(0).request;
    apply_linked_story(&output, &saved).unwrap();
}
#[test]
fn malformed_group_span_is_rejected_without_index_arithmetic_overflow() {
    let input = nested_group_fixture();
    let mut request = nested_request(&input);
    request.table_layout.as_mut().unwrap().cells[3].row_span = usize::MAX;
    assert!(validate_config(&request).is_err());
}
