//! Tagged-PDF semantic extraction.
//!
//! Tagged PDFs expose an authored logical structure tree (`/StructTreeRoot`)
//! whose `/StructElem` nodes point to page marked-content ranges by MCID. This
//! module walks that tree in authored order, resolves `(page, stream, MCID)` text from
//! the content streams, and emits a semantic JSON-ready tree. When no structure
//! tree is present it falls back to the geometric layout analyzer.

use std::collections::{BTreeSet, HashMap, HashSet};

use serde::Serialize;

use crate::analysis::layout::{LayoutBlock, LayoutConfig};
use crate::analysis::tables::Table;
use crate::engine::ContentEngine;
use crate::error::{Result, WellfriendError};
use crate::info::decode_pdf_text_string;
use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;
use crate::text::{
    text_role_from_tag, MarkedContentId, MarkedTextChunk, ReadingOrderReconstructor, TextChunk,
    TextDiagnostic, TextDiagnosticSeverity, TextRoleSource, TextStructureContext,
    TextStructureEntry,
};

const MAX_STRUCT_DEPTH: usize = 128;
const MIN_STRUCT_NODES: usize = 250_000;
const STRUCT_NODES_PER_PAGE: usize = 10_000;
const MAX_STRUCT_NODES: usize = 2_000_000;

fn semantic_structure_node_limit(page_count: usize) -> usize {
    page_count
        .saturating_mul(STRUCT_NODES_PER_PAGE)
        .clamp(MIN_STRUCT_NODES, MAX_STRUCT_NODES)
}

#[cfg(test)]
#[path = "semantic_scope_tests.rs"]
mod scope_tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticSource {
    TaggedPdf,
    GeometricFallback,
}

#[derive(Debug, Clone, Serialize)]
pub struct SemanticDocument {
    pub tagged: bool,
    pub source: SemanticSource,
    pub elements: Vec<SemanticElement>,
    pub tables: Vec<Table>,
}

impl SemanticDocument {
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for element in &self.elements {
            write_element_text(element, 0, &mut out);
        }
        out.trim_end().to_string()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SemanticElement {
    #[serde(rename = "type")]
    pub element_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_type: Option<String>,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<usize>,
    /// Bounding box of the element's own marked content in PDF user space
    /// `[x0, y0, x1, y1]` (y-up), recovered as the union of the text-chunk boxes
    /// for this element's MCIDs. `None` when the element has no resolvable
    /// marked text (e.g. a pure container, or a `Figure` whose content is a
    /// non-text XObject). Used by the document-model layer to give tagged
    /// elements geometry for ordering/linkage.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bbox: Option<[f64; 4]>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub mcids: Vec<SemanticMcid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery_evidence: Option<crate::semantic_intelligence::SemanticEvidenceKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery_confidence: Option<f32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<SemanticElement>,
}

impl SemanticElement {
    pub fn combined_text(&self) -> String {
        let mut parts = Vec::new();
        if !self.text.trim().is_empty() {
            parts.push(self.text.trim().to_string());
        }
        for child in &self.children {
            let child_text = child.combined_text();
            if !child_text.trim().is_empty() {
                parts.push(child_text);
            }
        }
        parts
            .join(" ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SemanticMcid {
    pub page: usize,
    pub mcid: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<(u32, u16)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_owner: Option<(u32, u16)>,
}

impl SemanticMcid {
    fn key(&self) -> McidKey {
        (
            self.page,
            MarkedContentId {
                mcid: self.mcid,
                stream: self.stream,
                stream_owner: self.stream_owner,
            },
        )
    }
}

type PageRef = (u32, u16);
type McidKey = (usize, MarkedContentId);
type McidTextMap = HashMap<McidKey, Vec<TextChunk>>;

pub fn extract_semantic_document(
    engine: &ContentEngine,
    pages: &[usize],
) -> Result<SemanticDocument> {
    let total = engine.page_count()?;
    let page_list: Vec<usize> = if pages.is_empty() {
        (1..=total).collect()
    } else {
        pages.to_vec()
    };
    for &page in &page_list {
        if page == 0 || page > total {
            return Err(WellfriendError::MalformedPdf(format!(
                "page {page} out of range (document has {total})"
            )));
        }
    }

    let catalog = engine.document().get_catalog()?;
    let Some(root_obj) = catalog.get("StructTreeRoot").cloned() else {
        return geometric_fallback(engine, &page_list);
    };

    let reader = engine.document().reader();
    let page_by_ref = page_ref_map(engine)?;
    let selected: BTreeSet<usize> = page_list.iter().copied().collect();
    let marked_text = collect_marked_text(engine, &page_list)?;

    let root = reader.resolve(root_obj)?;
    let Some(root_dict) = root.as_dict() else {
        return geometric_fallback(engine, &page_list);
    };

    let mut stream_owners: HashMap<(usize, PageRef), BTreeSet<Option<PageRef>>> = HashMap::new();
    for &(page, id) in marked_text.keys() {
        if let Some(stream) = id.stream {
            stream_owners
                .entry((page, stream))
                .or_default()
                .insert(id.stream_owner);
        }
    }
    let mut parser = StructParser {
        reader,
        page_refs: page_by_ref.iter().map(|(id, page)| (*page, *id)).collect(),
        page_by_ref,
        marked_text,
        stream_owners,
        appearance_owners: crate::annotation_appearance::AppearanceOwnerIndex::default(),
        visited: HashSet::new(),
        role_map: parse_role_map(root_dict),
        page_streams: engine
            .document()
            .get_pages()?
            .into_iter()
            .map(|page| (page.page_number, page.contents))
            .collect(),
        nodes: 0,
        node_limit: semantic_structure_node_limit(total),
    };
    let mut elements = Vec::new();
    if let Some(kids) = root_dict.get("K") {
        parser.parse_kids(kids, None, &mut elements, 0)?;
    }

    elements = elements
        .into_iter()
        .filter_map(|el| prune_for_pages(el, &selected))
        .collect();
    if elements.is_empty() {
        let recovery =
            crate::semantic_intelligence::recover_parenttree_semantics(engine, &page_list)?;
        let recovered =
            crate::semantic_intelligence::semantic_elements_from_parenttree_recovery(&recovery);
        if !recovered.is_empty() {
            let tables = collect_semantic_tables(&recovered);
            return Ok(SemanticDocument {
                tagged: true,
                source: SemanticSource::TaggedPdf,
                elements: recovered,
                tables,
            });
        }
    }

    let tables = collect_semantic_tables(&elements);
    Ok(SemanticDocument {
        tagged: true,
        source: SemanticSource::TaggedPdf,
        elements,
        tables,
    })
}

pub fn extract_text_structure_context(
    engine: &ContentEngine,
    pages: &[usize],
    max_nodes: usize,
    max_mcids: usize,
) -> Result<TextStructureContext> {
    let catalog = engine.document().get_catalog()?;
    if catalog.get("StructTreeRoot").is_none() {
        return Ok(TextStructureContext::empty());
    }

    let document = extract_semantic_document(engine, pages)?;
    if !document.tagged {
        return Ok(TextStructureContext::empty());
    }

    let mut context = TextStructureContext::empty();
    let mut seen = HashSet::new();
    for element in &document.elements {
        flatten_structure_element(element, "", max_nodes, max_mcids, &mut seen, &mut context);
        if context.capped {
            break;
        }
    }
    Ok(context)
}

fn flatten_structure_element(
    element: &SemanticElement,
    parent_path: &str,
    max_nodes: usize,
    max_mcids: usize,
    seen: &mut HashSet<McidKey>,
    context: &mut TextStructureContext,
) {
    if context.entries.len() >= max_mcids || context.entries.len() >= max_nodes {
        context.capped = true;
        context.diagnostics.push(TextDiagnostic {
            code: "text.structure.cap".to_string(),
            severity: TextDiagnosticSeverity::Warning,
            page: element.page,
            message: "structure-to-MCID mapping hit configured cap".to_string(),
        });
        return;
    }
    let path = if parent_path.is_empty() {
        element.element_type.clone()
    } else {
        format!("{parent_path}/{}", element.element_type)
    };
    let original_role = element
        .original_type
        .clone()
        .unwrap_or_else(|| element.element_type.clone());
    let authored_role_source = if element.original_type.is_some() {
        TextRoleSource::RoleMap
    } else {
        TextRoleSource::Tagged
    };
    use crate::semantic_intelligence::SemanticEvidenceKind;
    let role_source = match element.recovery_evidence {
        None | Some(SemanticEvidenceKind::SpecDerivedStructure) => authored_role_source,
        Some(
            SemanticEvidenceKind::RepairedStructure
            | SemanticEvidenceKind::InferredStructure
            | SemanticEvidenceKind::ModelProposed,
        ) => TextRoleSource::Heuristic,
        _ => TextRoleSource::Unknown,
    };
    for mcid in &element.mcids {
        if context.entries.len() >= max_mcids {
            context.capped = true;
            context.diagnostics.push(TextDiagnostic {
                code: "text.structure.mcid_cap".to_string(),
                severity: TextDiagnosticSeverity::Warning,
                page: Some(mcid.page),
                message: "MCID mapping hit configured cap".to_string(),
            });
            return;
        }
        if !seen.insert(mcid.key()) {
            context.diagnostics.push(TextDiagnostic {
                code: "text.structure.duplicate_mcid".to_string(),
                severity: TextDiagnosticSeverity::Warning,
                page: Some(mcid.page),
                message: format!(
                    "duplicate StructTree mapping for MCID {} in stream {:?}",
                    mcid.mcid, mcid.stream
                ),
            });
        }
        if element.text.trim().is_empty()
            && element.actual_text.is_none()
            && element.alt_text.is_none()
        {
            context.diagnostics.push(TextDiagnostic {
                code: "text.structure.empty_mcid".to_string(),
                severity: TextDiagnosticSeverity::Info,
                page: Some(mcid.page),
                message: format!("StructTree MCID {} has no resolved text", mcid.mcid),
            });
        }
        context.entries.push(TextStructureEntry {
            page: mcid.page,
            mcid: mcid.mcid,
            stream: mcid.stream,
            stream_owner: mcid.stream_owner,
            role: text_role_from_tag(&element.element_type),
            normalized_role: element.element_type.clone(),
            original_role: original_role.clone(),
            role_source,
            confidence: element.recovery_confidence.unwrap_or(
                if role_source == TextRoleSource::Tagged {
                    0.94
                } else {
                    0.82
                },
            ),
            artifact: element.element_type.eq_ignore_ascii_case("Artifact"),
            actual_text: element.actual_text.clone(),
            alt_text: element.alt_text.clone(),
            lang: element.lang.clone(),
            struct_path: Some(path.clone()),
        });
    }
    for child in &element.children {
        flatten_structure_element(child, &path, max_nodes, max_mcids, seen, context);
        if context.capped {
            break;
        }
    }
}

fn geometric_fallback(engine: &ContentEngine, pages: &[usize]) -> Result<SemanticDocument> {
    let mut children = Vec::new();
    for &page in pages {
        let layout = engine.analyze_page_layout_with(page, &LayoutConfig::default())?;
        children.extend(
            layout
                .blocks
                .into_iter()
                .map(|block| block_to_element(page, block)),
        );
    }

    Ok(SemanticDocument {
        tagged: false,
        source: SemanticSource::GeometricFallback,
        tables: Vec::new(),
        elements: vec![SemanticElement {
            element_type: "Document".to_string(),
            original_type: None,
            text: String::new(),
            alt_text: None,
            actual_text: None,
            lang: None,
            page: None,
            bbox: None,
            mcids: Vec::new(),
            recovery_evidence: None,
            recovery_confidence: None,
            children,
        }],
    })
}

fn block_to_element(page: usize, block: LayoutBlock) -> SemanticElement {
    SemanticElement {
        element_type: "P".to_string(),
        original_type: None,
        text: block.text(),
        alt_text: None,
        actual_text: None,
        lang: None,
        page: Some(page),
        bbox: Some([block.bbox.x0, block.bbox.y0, block.bbox.x1, block.bbox.y1]),
        mcids: Vec::new(),
        recovery_evidence: None,
        recovery_confidence: None,
        children: Vec::new(),
    }
}

struct StructParser<'a> {
    reader: &'a PdfReader,
    page_by_ref: HashMap<PageRef, usize>,
    page_refs: HashMap<usize, PageRef>,
    marked_text: McidTextMap,
    stream_owners: HashMap<(usize, PageRef), BTreeSet<Option<PageRef>>>,
    appearance_owners: crate::annotation_appearance::AppearanceOwnerIndex,
    visited: HashSet<PageRef>,
    role_map: HashMap<String, String>,
    page_streams: HashMap<usize, Vec<PageRef>>,
    nodes: usize,
    node_limit: usize,
}

impl<'a> StructParser<'a> {
    fn visit(&mut self, depth: usize) -> Result<()> {
        crate::cancel::check_current_cancel("semantic structure traversal")?;
        self.nodes = self.nodes.checked_add(1).ok_or_else(|| {
            WellfriendError::ResourceLimit("structure node counter overflow".into())
        })?;
        if depth > MAX_STRUCT_DEPTH || self.nodes > self.node_limit {
            return Err(WellfriendError::ResourceLimit(
                "semantic structure traversal limit exceeded".into(),
            ));
        }
        Ok(())
    }
    fn parse_kids(
        &mut self,
        object: &PdfObject,
        inherited_page: Option<usize>,
        out: &mut Vec<SemanticElement>,
        depth: usize,
    ) -> Result<()> {
        // Count only traversable structure containers.  MCID integers and
        // other scalar leaf values are handled by their owning StructElem and
        // must not consume the graph-node budget independently.  Large valid
        // tagged forms routinely contain hundreds of thousands of scalar /K
        // leaves even though their object graph remains bounded.
        if matches!(
            object,
            PdfObject::Array(_) | PdfObject::Reference { .. } | PdfObject::Dictionary(_)
        ) {
            self.visit(depth)?;
        } else {
            return Ok(());
        }
        if depth > MAX_STRUCT_DEPTH {
            return Err(WellfriendError::MalformedPdf(
                "structure tree exceeded depth limit".to_string(),
            ));
        }
        match object {
            PdfObject::Array(items) => {
                for item in items {
                    self.parse_kids(item, inherited_page, out, depth + 1)?;
                }
            }
            PdfObject::Reference { number, generation } => {
                let id = (*number, *generation);
                if !self.visited.insert(id) {
                    return Err(WellfriendError::MalformedPdf(format!(
                        "cyclic structure reference {number} {generation}"
                    )));
                }
                let resolved = self.reader.get_and_resolve(*number, *generation)?;
                self.parse_kids(&resolved, inherited_page, out, depth + 1)?;
                self.visited.remove(&id);
            }
            PdfObject::Dictionary(dict)
                if dict.get_name("S").is_some()
                    || matches!(dict.get_name("Type"), Some("StructElem")) =>
            {
                out.push(self.parse_element(dict, inherited_page, depth + 1)?);
            }
            _ => {}
        }
        Ok(())
    }

    fn parse_element(
        &mut self,
        dict: &PdfDictionary,
        inherited_page: Option<usize>,
        depth: usize,
    ) -> Result<SemanticElement> {
        self.visit(depth)?;
        if depth > MAX_STRUCT_DEPTH {
            return Err(WellfriendError::MalformedPdf(
                "structure tree exceeded depth limit".to_string(),
            ));
        }
        let original_type = dict.get_name("S").unwrap_or("Span").to_string();
        let element_type = self
            .role_map
            .get(&original_type)
            .cloned()
            .unwrap_or_else(|| original_type.clone());
        let original_type_field = (original_type != element_type).then_some(original_type);
        let page = self.element_page(dict, inherited_page)?;
        let alt_text = dict.get("Alt").and_then(pdf_text_value);
        let actual_text = dict.get("ActualText").and_then(pdf_text_value);
        let lang = dict.get("Lang").and_then(pdf_text_value);

        let mut mcids = Vec::new();
        let mut children = Vec::new();
        if let Some(kids) = dict.get("K") {
            self.parse_element_kids(kids, page, &mut mcids, &mut children, depth + 1)?;
        }

        let text = match &actual_text {
            Some(actual) => actual.clone(),
            None => text_for_mcids(&mcids, &self.marked_text),
        };
        let bbox = bbox_for_mcids(&mcids, &self.marked_text);
        let resolved_pages = mcids
            .iter()
            .filter(|id| self.marked_text.contains_key(&id.key()))
            .map(|id| id.page)
            .collect::<BTreeSet<_>>();
        let page = match resolved_pages.len() {
            0 => page,
            1 => resolved_pages.first().copied(),
            _ => None,
        };

        Ok(SemanticElement {
            element_type,
            original_type: original_type_field,
            text,
            alt_text,
            actual_text,
            lang,
            page,
            bbox,
            mcids,
            recovery_evidence: None,
            recovery_confidence: None,
            children,
        })
    }

    fn parse_element_kids(
        &mut self,
        object: &PdfObject,
        inherited_page: Option<usize>,
        mcids: &mut Vec<SemanticMcid>,
        children: &mut Vec<SemanticElement>,
        depth: usize,
    ) -> Result<()> {
        self.visit(depth)?;
        if depth > MAX_STRUCT_DEPTH {
            return Err(WellfriendError::MalformedPdf(
                "structure tree exceeded depth limit".to_string(),
            ));
        }
        match object {
            PdfObject::Integer(mcid) => {
                if let Some(page) = inherited_page {
                    if *mcid < 0 {
                        return Err(WellfriendError::MalformedPdf(
                            "negative structure MCID".into(),
                        ));
                    }
                    mcids.push(SemanticMcid {
                        page,
                        mcid: *mcid,
                        stream: None,
                        stream_owner: None,
                    });
                }
            }
            PdfObject::Array(items) => {
                for item in items {
                    self.parse_element_kids(item, inherited_page, mcids, children, depth + 1)?;
                }
            }
            PdfObject::Reference { number, generation } => {
                let id = (*number, *generation);
                if !self.visited.insert(id) {
                    return Err(WellfriendError::MalformedPdf(format!(
                        "cyclic structure kid {number} {generation}"
                    )));
                }
                let resolved = self.reader.get_and_resolve(*number, *generation)?;
                self.parse_element_kids(&resolved, inherited_page, mcids, children, depth + 1)?;
                self.visited.remove(&id);
            }
            PdfObject::Dictionary(dict) => {
                if dict.get_name("S").is_some()
                    || matches!(dict.get_name("Type"), Some("StructElem"))
                {
                    children.push(self.parse_element(dict, inherited_page, depth + 1)?);
                } else if dict.get_name("Type") == Some("MCR") || dict.get("MCID").is_some() {
                    let value = dict
                        .get("MCID")
                        .ok_or_else(|| WellfriendError::MalformedPdf("MCR has no MCID".into()))?;
                    let mcid = self
                        .reader
                        .resolve(value.clone())?
                        .as_integer()
                        .ok_or_else(|| {
                            WellfriendError::MalformedPdf("MCR MCID is not an integer".into())
                        })?;
                    let page = self.element_page(dict, inherited_page)?;
                    if let Some(page) = page {
                        if mcid < 0 {
                            return Err(WellfriendError::MalformedPdf(
                                "negative structure MCID".into(),
                            ));
                        }
                        let (stream, stream_owner) = self.mcr_stream(dict, page)?;
                        mcids.push(SemanticMcid {
                            page,
                            mcid,
                            stream,
                            stream_owner,
                        });
                    } else {
                        return Err(WellfriendError::MalformedPdf(
                            "MCR has no page context".into(),
                        ));
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn mcr_stream(
        &mut self,
        dict: &PdfDictionary,
        page: usize,
    ) -> Result<(Option<PageRef>, Option<PageRef>)> {
        let Some(value) = dict.get("Stm").filter(|value| !value.is_null()) else {
            if dict.get("StmOwn").is_some_and(|value| !value.is_null()) {
                return Err(WellfriendError::MalformedPdf(
                    "MCR StmOwn has no Stm".into(),
                ));
            }
            return Ok((None, None));
        };
        let reference = value.as_reference().ok_or_else(|| {
            WellfriendError::MalformedPdf("MCR Stm must be an indirect stream reference".into())
        })?;
        let object = self.reader.get_object(reference.0, reference.1)?;
        if object.as_stream().is_none() {
            return Err(WellfriendError::MalformedPdf(
                "MCR Stm does not reference a stream".into(),
            ));
        }
        if let Some(value) = dict.get("StmOwn").filter(|value| !value.is_null()) {
            let owner = value.as_reference().ok_or_else(|| {
                WellfriendError::MalformedPdf("MCR StmOwn must be indirect".into())
            })?;
            let page_ref =
                self.page_refs.get(&page).copied().ok_or_else(|| {
                    WellfriendError::MalformedPdf("MCR page is not reachable".into())
                })?;
            self.appearance_owners
                .validate(self.reader, page_ref, owner, reference)?;
            return Ok((Some(reference), Some(owner)));
        }
        // A page Contents array is one marked-content namespace. Explicitly
        // naming one of its streams does not invent a second namespace.
        if self
            .page_streams
            .get(&page)
            .is_some_and(|streams| streams.contains(&reference))
        {
            return Ok((None, None));
        }
        // An omitted optional StmOwn is safe to infer only when this page's
        // extracted content has exactly one owning namespace for the stream.
        let owners = self.stream_owners.get(&(page, reference));
        if owners.is_some_and(|owners| owners.len() > 1) {
            return Err(WellfriendError::UnsupportedFeature(
                "MCR without StmOwn has multiple appearance/content owners".into(),
            ));
        }
        Ok((
            Some(reference),
            owners
                .and_then(|owners| owners.iter().next().copied())
                .flatten(),
        ))
    }

    fn element_page(
        &self,
        dict: &PdfDictionary,
        inherited: Option<usize>,
    ) -> Result<Option<usize>> {
        match dict.get("Pg") {
            None | Some(PdfObject::Null) => Ok(inherited),
            Some(value) => page_from_object(value, &self.page_by_ref)
                .map(Some)
                .ok_or_else(|| {
                    WellfriendError::MalformedPdf(
                        "structure Pg does not identify a reachable page".into(),
                    )
                }),
        }
    }
}

fn page_ref_map(engine: &ContentEngine) -> Result<HashMap<PageRef, usize>> {
    let mut out = HashMap::new();
    for page in engine.document().get_pages()? {
        out.insert(
            (page.object_number, page.generation_number),
            page.page_number,
        );
    }
    Ok(out)
}

fn page_from_object(object: &PdfObject, page_by_ref: &HashMap<PageRef, usize>) -> Option<usize> {
    object
        .as_reference()
        .and_then(|id| page_by_ref.get(&id).copied())
}

fn pdf_text_value(object: &PdfObject) -> Option<String> {
    match object {
        PdfObject::String(bytes) => Some(decode_pdf_text_string(bytes)),
        PdfObject::Name(name) => Some(name.clone()),
        _ => None,
    }
}

fn parse_role_map(root: &PdfDictionary) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Some(map) = root.get("RoleMap").and_then(PdfObject::as_dict) else {
        return out;
    };
    for (from, to) in map.iter() {
        if let Some(role) = to.as_name() {
            out.insert(from.clone(), role.to_string());
        }
    }
    out
}

fn collect_marked_text(engine: &ContentEngine, pages: &[usize]) -> Result<McidTextMap> {
    let mut out: McidTextMap = HashMap::new();
    for page in pages.iter().copied().collect::<BTreeSet<_>>() {
        for marked in engine.collect_page_marked_text_chunks_including_appearances(page)? {
            push_marked_chunk(page, marked, &mut out);
        }
    }
    Ok(out)
}

fn push_marked_chunk(page: usize, marked: MarkedTextChunk, out: &mut McidTextMap) {
    if let Some(id) = marked.marked_content_id() {
        out.entry((page, id)).or_default().push(marked.chunk);
    }
}

fn text_for_mcids(mcids: &[SemanticMcid], text_map: &McidTextMap) -> String {
    let mut chunks = Vec::new();
    let mut parts = Vec::new();
    let mut current_page = None;
    for id in mcids {
        if let Some(found) = text_map.get(&id.key()) {
            if current_page.is_some_and(|page| page != id.page) && !chunks.is_empty() {
                parts.push(chunks_to_text(std::mem::take(&mut chunks)));
            }
            current_page = Some(id.page);
            chunks.extend(found.iter().cloned());
        }
    }
    if !chunks.is_empty() {
        parts.push(chunks_to_text(chunks));
    }
    parts.join("\n")
}

/// Union of the text-chunk boxes for an element's MCIDs, in user space (y-up).
/// Each chunk's box is `[x, x+width] × [y, y+font_size]` (matching the layout
/// analyzer). Returns `None` when no MCID resolves to any text. The union is
/// min/max, so the `HashMap` lookup order does not affect the result.
fn bbox_for_mcids(mcids: &[SemanticMcid], text_map: &McidTextMap) -> Option<[f64; 4]> {
    let mut x0 = f64::INFINITY;
    let mut y0 = f64::INFINITY;
    let mut x1 = f64::NEG_INFINITY;
    let mut y1 = f64::NEG_INFINITY;
    let mut any = false;
    let mut page = None;
    for id in mcids {
        let Some(found) = text_map.get(&id.key()) else {
            continue;
        };
        // Coordinates on different pages do not form one geometric region.
        if page.is_some_and(|page| page != id.page) {
            return None;
        }
        page = Some(id.page);
        for c in found {
            if c.text.trim().is_empty() {
                continue;
            }
            let fs = if c.font_size > 0.0 { c.font_size } else { 1.0 };
            x0 = x0.min(c.x);
            y0 = y0.min(c.y);
            x1 = x1.max(c.x + c.width.max(0.0));
            y1 = y1.max(c.y + fs);
            any = true;
        }
    }
    if any && x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite() {
        Some([x0, y0, x1, y1])
    } else {
        None
    }
}

fn chunks_to_text(chunks: Vec<TextChunk>) -> String {
    if chunks.is_empty() {
        return String::new();
    }
    let mut reconstructor = ReadingOrderReconstructor::new();
    reconstructor.detect_columns = false;
    reconstructor
        .reconstruct(chunks)
        .into_iter()
        .map(|line| line.text)
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn prune_for_pages(
    element: SemanticElement,
    selected: &BTreeSet<usize>,
) -> Option<SemanticElement> {
    let mut element = element;
    element.children = element
        .children
        .into_iter()
        .filter_map(|child| prune_for_pages(child, selected))
        .collect();
    element.mcids.retain(|id| selected.contains(&id.page));

    let direct_selected = element
        .page
        .map(|page| selected.contains(&page))
        .unwrap_or(false)
        || element.mcids.iter().any(|id| selected.contains(&id.page));
    if direct_selected || !element.children.is_empty() || selected.is_empty() {
        Some(element)
    } else {
        None
    }
}

fn collect_semantic_tables(elements: &[SemanticElement]) -> Vec<Table> {
    let mut out = Vec::new();
    for element in elements {
        collect_semantic_tables_from(element, &mut out);
    }
    out
}

fn collect_semantic_tables_from(element: &SemanticElement, out: &mut Vec<Table>) {
    if element.element_type == "Table" {
        if let Some(table) = table_from_element(element) {
            out.push(table);
        }
    }
    for child in &element.children {
        collect_semantic_tables_from(child, out);
    }
}

fn table_from_element(element: &SemanticElement) -> Option<Table> {
    let rows: Vec<Vec<(String, bool)>> = element
        .children
        .iter()
        .filter(|child| child.element_type == "TR")
        .map(|row| {
            row.children
                .iter()
                .filter(|cell| cell.element_type == "TH" || cell.element_type == "TD")
                .map(|cell| {
                    (
                        cell.combined_text(),
                        cell.element_type.eq_ignore_ascii_case("TH"),
                    )
                })
                .collect::<Vec<_>>()
        })
        .filter(|row| !row.is_empty())
        .collect();

    if rows.is_empty() {
        None
    } else {
        Some(Table::from_semantic_rows(rows))
    }
}

fn write_element_text(element: &SemanticElement, depth: usize, out: &mut String) {
    let indent = "  ".repeat(depth);
    let display = element
        .alt_text
        .as_ref()
        .filter(|_| element.element_type == "Figure")
        .or_else(|| {
            if element.text.trim().is_empty() {
                None
            } else {
                Some(&element.text)
            }
        });

    if let Some(text) = display {
        out.push_str(&indent);
        out.push_str(&element.element_type);
        out.push_str(": ");
        out.push_str(text.trim());
        out.push('\n');
    } else if !element.children.is_empty() {
        out.push_str(&indent);
        out.push_str(&element.element_type);
        out.push('\n');
    }

    for child in &element.children {
        write_element_text(child, depth + 1, out);
    }
}
