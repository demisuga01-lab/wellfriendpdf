//! Resolve C/ClassMap before A, retaining per-attachment revision numbers.
//! Reflow materializes attributes on the selected owner, never in shared classes.
use super::*;

pub(super) struct Entry {
    pub value: PdfObject,
    pub revision: i64,
}
pub(super) struct Budget {
    slots: usize,
    bytes: usize,
}
impl Default for Budget {
    fn default() -> Self {
        Self {
            slots: 1_000_000,
            bytes: 64 * 1024 * 1024,
        }
    }
}
pub(super) struct Resolver {
    classes: PdfDictionary,
}
impl Resolver {
    pub(super) fn new(store: &Store<'_>) -> Result<Self> {
        let catalog = store.dict(
            store
                .reader
                .root_reference()
                .ok_or_else(|| fail("missing structure catalog"))?,
        )?;
        let root = catalog
            .get("StructTreeRoot")
            .map(|v| store.resolve(v))
            .transpose()?;
        let classes = root
            .as_ref()
            .and_then(PdfObject::as_dict)
            .and_then(|d| d.get("ClassMap"))
            .map(|v| store.resolve(v))
            .transpose()?;
        Ok(Self {
            classes: match classes {
                None | Some(PdfObject::Null) => PdfDictionary::empty(),
                Some(PdfObject::Dictionary(d)) => d,
                _ => return Err(fail("invalid structure ClassMap")),
            },
        })
    }
    pub(super) fn entries(
        &self,
        store: &Store<'_>,
        dict: &PdfDictionary,
        budget: &mut Budget,
    ) -> Result<Vec<Entry>> {
        fn list(store: &Store<'_>, value: Option<&PdfObject>) -> Result<Vec<PdfObject>> {
            let Some(value) = value else {
                return Ok(Vec::new());
            };
            Ok(match store.resolve(value)? {
                PdfObject::Null => Vec::new(),
                PdfObject::Array(v) => v,
                _ => vec![value.clone()], // retain indirect stream identity
            })
        }
        fn pairs(store: &Store<'_>, values: Vec<PdfObject>) -> Result<Vec<(PdfObject, i64)>> {
            if values.len() > 8192 {
                return Err(fail("attribute attachment budget exceeded"));
            }
            let mut out = Vec::new();
            let mut values = values.into_iter().peekable();
            while let Some(value) = values.next() {
                if matches!(store.resolve(&value)?, PdfObject::Integer(_)) {
                    return Err(fail("attribute revision lacks an owner"));
                }
                let revision = if let Some(next) = values.peek() {
                    match store.resolve(next)? {
                        PdfObject::Integer(n) if n >= 0 => {
                            values.next();
                            n
                        }
                        PdfObject::Integer(_) => return Err(fail("negative attribute revision")),
                        _ => 0,
                    }
                } else {
                    0
                };
                out.push((value, revision));
            }
            Ok(out)
        }
        let mut pending = Vec::new();
        for (class, revision) in pairs(store, list(store, dict.get("C"))?)? {
            let PdfObject::Name(name) = store.resolve(&class)? else {
                return Err(fail("attribute class is not a name"));
            };
            // ISO 32000-2 errata: a missing class contributes no attributes.
            for value in list(store, self.classes.get(&name))? {
                if pending.len() >= 4096 {
                    return Err(fail("effective class attribute budget exceeded"));
                }
                pending.push((value, revision));
            }
        }
        let direct = pairs(store, list(store, dict.get("A"))?)?;
        if pending.len().saturating_add(direct.len()) > 4096 {
            return Err(fail("effective attribute count exceeded"));
        }
        pending.extend(direct);
        let mut out = Vec::with_capacity(pending.len());
        for (value, revision) in pending {
            crate::cancel::check_current_cancel("structure class attribute resolution")?;
            budget.slots = budget
                .slots
                .checked_sub(1)
                .ok_or_else(|| fail("aggregate attribute budget exceeded"))?;
            let (resolved, id) = store.resolve_identity(&value)?;
            let d = match &resolved {
                PdfObject::Dictionary(d) => d,
                PdfObject::Stream { dict, raw } => {
                    if raw.len() > 16 * 1024 * 1024 {
                        return Err(fail("attribute stream too large"));
                    }
                    budget.bytes = budget
                        .bytes
                        .checked_sub(raw.len())
                        .ok_or_else(|| fail("attribute stream aggregate budget exceeded"))?;
                    if id.is_none() {
                        return Err(fail("attribute streams must be indirect"));
                    }
                    dict
                }
                _ => return Err(fail("attribute object is not a dictionary or stream")),
            };
            if d.get_name("O").is_none() {
                return Err(fail("attribute object lacks its owner"));
            }
            out.push(Entry {
                value: id.map(reference).unwrap_or(resolved),
                revision,
            });
        }
        Ok(out)
    }
    pub(super) fn rewrite(
        &self,
        store: &mut Store<'_>,
        dict: &mut PdfDictionary,
        table: Option<PdfDictionary>,
        budget: &mut Budget,
    ) -> Result<()> {
        self.rewrite_impl(store, dict, table, None, budget)
    }
    pub(super) fn rewrite_flow(
        &self,
        store: &mut Store<'_>,
        dict: &mut PdfDictionary,
        table: Option<PdfDictionary>,
        mode: crate::fonts::WritingMode,
        rtl: bool,
        budget: &mut Budget,
    ) -> Result<()> {
        let writing_mode = match mode {
            crate::fonts::WritingMode::HorizontalTb => Some(if rtl { "RlTb" } else { "LrTb" }),
            crate::fonts::WritingMode::VerticalRl => Some("TbRl"),
            crate::fonts::WritingMode::VerticalLr => Some("TbLr"),
        };
        self.rewrite_impl(store, dict, table, writing_mode, budget)
    }
    fn rewrite_impl(
        &self,
        store: &mut Store<'_>,
        dict: &mut PdfDictionary,
        table: Option<PdfDictionary>,
        writing_mode: Option<&str>,
        budget: &mut Budget,
    ) -> Result<()> {
        let revision = match dict.get("R").map(|v| store.resolve(v)).transpose()? {
            None | Some(PdfObject::Null) => 0,
            Some(PdfObject::Integer(n)) if n >= 0 => n,
            _ => return Err(fail("invalid structure revision")),
        }
        .checked_add(1)
        .ok_or_else(|| fail("structure revision exhausted"))?;
        let entries = self.entries(store, dict, budget)?;
        let mut values = Vec::new();
        for entry in entries {
            let mut object = store.resolve(&entry.value)?;
            let d = match &mut object {
                PdfObject::Dictionary(d) => d,
                PdfObject::Stream { dict, .. } => dict,
                _ => return Err(fail("invalid resolved attribute object")),
            };
            let remove: &[&str] = match d.get_name("O") {
                Some("Layout") if writing_mode.is_some() => {
                    &["BBox", "Width", "Height", "WritingMode"]
                }
                Some("Layout") => &["BBox", "Width", "Height"],
                Some("Table")
                    if table
                        .as_ref()
                        .is_some_and(|d| d.get_name("O") == Some("Table")) =>
                {
                    &["RowSpan", "ColSpan", "Scope", "Headers"]
                }
                _ => &[],
            };
            let mut changed = false;
            for name in remove {
                if d.contains_key(name) {
                    d.remove(name);
                    changed = true;
                }
            }
            if changed && d.iter().all(|(name, _)| name == "O" || name == "NS") {
                continue;
            }
            let value = if changed {
                if matches!(&object, PdfObject::Stream { .. }) {
                    reference(store.add(object)?)
                } else {
                    object
                }
            } else {
                entry.value
            };
            values.push(value);
            values.push(PdfObject::Integer(entry.revision));
        }
        if let Some(table) = table {
            values.push(PdfObject::Dictionary(table));
            values.push(PdfObject::Integer(revision));
        }
        if let Some(mode) = writing_mode {
            let mut layout = PdfDictionary::empty();
            layout.insert("O", PdfObject::Name("Layout".into()));
            layout.insert("WritingMode", PdfObject::Name(mode.into()));
            values.push(PdfObject::Dictionary(layout));
            values.push(PdfObject::Integer(revision));
        }
        dict.remove("C");
        if values.is_empty() {
            dict.remove("A");
        } else {
            dict.insert("A", PdfObject::Array(values));
        }
        // Unknown attribute owners retain old revisions, signalling that this
        // content change has not re-qualified their application-specific data.
        dict.insert("R", PdfObject::Integer(revision));
        Ok(())
    }
    /// Winning dictionary properties for reference checks. A later attribute
    /// object wins per owner/namespace/property; obsolete class Headers must not
    /// falsely prevent deletion after an explicit A override removed that edge.
    pub(super) fn effective(
        &self,
        store: &Store<'_>,
        dict: &PdfDictionary,
        budget: &mut Budget,
    ) -> Result<Vec<PdfObject>> {
        let mut owners = BTreeMap::<(String, Option<ObjectRef>), PdfDictionary>::new();
        for entry in self.entries(store, dict, budget)? {
            let (d, stream) = match store.resolve(&entry.value)? {
                PdfObject::Dictionary(d) => (d, false),
                PdfObject::Stream { dict, .. } => (dict, true),
                _ => return Err(fail("invalid effective attribute object")),
            };
            let owner = d
                .get_name("O")
                .ok_or_else(|| fail("missing attribute owner"))?
                .to_owned();
            let namespace = match d.get("NS") {
                None | Some(PdfObject::Null) => None,
                Some(value) => Some(
                    value
                        .as_reference()
                        .ok_or_else(|| fail("attribute namespace needs an indirect identity"))?,
                ),
            };
            let target = owners.entry((owner, namespace)).or_default();
            for (key, value) in d.iter() {
                if !stream || !matches!(key.as_str(), "Length" | "Filter" | "DecodeParms") {
                    target.insert(key.clone(), value.clone());
                }
            }
        }
        Ok(owners.into_values().map(PdfObject::Dictionary).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stream_attributes_are_cloned_per_owner_with_revision_evidence() {
        let bytes = crate::linked_stories::tables::tests::fixture(false);
        let engine = ContentEngine::open_bytes(bytes).unwrap();
        let mut store = Store::new(engine.document().reader());
        let mut attr = PdfDictionary::empty();
        attr.insert("O", PdfObject::Name("Layout".into()));
        attr.insert("BBox", PdfObject::Array(vec![PdfObject::Integer(0); 4]));
        attr.insert("Padding", PdfObject::Integer(2));
        let raw = b"opaque application data".to_vec();
        attr.insert("Length", PdfObject::Integer(raw.len() as i64));
        let original = store
            .add(PdfObject::Stream {
                dict: attr,
                raw: raw.clone(),
            })
            .unwrap();
        let mut node = PdfDictionary::empty();
        node.insert("R", PdfObject::Integer(7));
        node.insert(
            "A",
            PdfObject::Array(vec![reference(original), PdfObject::Integer(7)]),
        );
        let resolver = Resolver::new(&store).unwrap();
        resolver
            .rewrite(&mut store, &mut node, None, &mut Budget::default())
            .unwrap();
        let copy = node.get("A").unwrap().as_array().unwrap()[0]
            .as_reference()
            .unwrap();
        assert_ne!(copy, original);
        assert_eq!(node.get_integer("R"), Some(8));
        let PdfObject::Stream { dict, raw: copied } = store.get(copy).unwrap() else {
            panic!("lost attribute stream")
        };
        assert_eq!(copied, raw);
        assert!(!dict.contains_key("BBox"));
        assert_eq!(dict.get_integer("Padding"), Some(2));
        let PdfObject::Stream { dict, .. } = store.get(original).unwrap() else {
            panic!("lost shared stream")
        };
        assert!(dict.contains_key("BBox"));
        let entries = resolver
            .entries(&store, &node, &mut Budget::default())
            .unwrap();
        assert_eq!(entries[0].revision, 7);
    }
    #[test]
    fn repeated_table_attribute_rewrites_do_not_accumulate_empty_owners() {
        let bytes = crate::linked_stories::tables::tests::fixture(false);
        let engine = ContentEngine::open_bytes(bytes).unwrap();
        let mut store = Store::new(engine.document().reader());
        let resolver = Resolver::new(&store).unwrap();
        let mut node = PdfDictionary::empty();
        for revision in 1..=20 {
            let mut attr = PdfDictionary::empty();
            attr.insert("O", PdfObject::Name("Table".into()));
            attr.insert("RowSpan", PdfObject::Integer(1));
            resolver
                .rewrite(&mut store, &mut node, Some(attr), &mut Budget::default())
                .unwrap();
            let entries = resolver
                .entries(&store, &node, &mut Budget::default())
                .unwrap();
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].revision, revision);
            assert_eq!(node.get_integer("R"), Some(revision));
        }
    }
}
