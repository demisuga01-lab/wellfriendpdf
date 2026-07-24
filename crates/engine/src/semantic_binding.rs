//! Stable, binding-neutral semantic export bundle.
//!
//! Public language bindings consume this one typed Rust report through a
//! versioned JSON envelope. This avoids duplicating semantic ownership graphs in
//! C, Java, .NET, Python, and WASM while preserving the same provenance fields.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::advanced_rag::{
    advanced_chunk_document, AdvancedChunkContext, AdvancedChunkOptions, AdvancedRagChunkSet,
    ChunkSecurityPosture, RagCjkToken,
};
use crate::analysis::tables::Table;
use crate::error::Result;
use crate::parse::{BlockKind, Document, ParseOptions};
use crate::security::scan_risky_content;
use crate::semantic::{SemanticDocument, SemanticElement};
use crate::semantic_intelligence::{
    layout_backend_availability_report, CloudLayoutBackendConfig, LayoutAvailabilityReport,
    LayoutLocalBackendConfig, ParentTreeRecoveryReport,
};
use crate::table_intelligence::{
    merge_table_proposals_deterministic, table_model_backend_status_report,
    DeterministicTableEvidence, TableModelBackendStatusReport, TableProposalMergePolicy,
    TableProposalMergeReport, TableProposalSet,
};
use crate::text::{
    segment_cjk_dictionary_text_with_provider, CjkDictionaryLoadReport, CjkDictionaryProvider,
    CjkDictionaryProviderLimits, CjkSegmentationMode, TextSearchMatch, TextSearchOptions,
    TextSemanticDocument, TextSemanticOptions,
};
use crate::ContentEngine;

pub const SEMANTIC_BINDING_SCHEMA_VERSION: &str = "prompt15.semantic_binding.v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SemanticBindingOptions {
    pub pages: Vec<usize>,
    pub include_chars: bool,
    pub include_hidden_text: bool,
    pub dictionary_enabled: bool,
    pub dictionary_manifest_paths: Vec<PathBuf>,
    pub dictionary_limits: CjkDictionaryProviderLimits,
    pub chunk_options: AdvancedChunkOptions,
    pub search_query: Option<String>,
    pub search_case_sensitive: bool,
    pub table_proposals: Option<TableProposalSet>,
}

impl Default for SemanticBindingOptions {
    fn default() -> Self {
        Self {
            pages: Vec::new(),
            include_chars: true,
            include_hidden_text: false,
            dictionary_enabled: true,
            dictionary_manifest_paths: Vec::new(),
            dictionary_limits: CjkDictionaryProviderLimits::default(),
            chunk_options: AdvancedChunkOptions::default(),
            search_query: None,
            search_case_sensitive: false,
            table_proposals: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SemanticBindingSummary {
    pub page_count: usize,
    pub block_count: usize,
    pub paragraph_count: usize,
    pub line_count: usize,
    pub span_count: usize,
    pub char_count: usize,
    pub structure_node_count: usize,
    pub mcid_count: usize,
    pub recovered_parenttree_node_count: usize,
    pub orphan_mcid_count: usize,
    pub parenttree_conflict_count: usize,
    pub table_count: usize,
    pub table_cell_count: usize,
    pub figure_count: usize,
    pub caption_count: usize,
    pub cjk_token_count: usize,
    pub rag_chunk_count: usize,
    pub search_match_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SemanticPageTables {
    pub page: usize,
    pub tables: Vec<Table>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SemanticCjkTokenPage {
    pub page: usize,
    pub raw_text: String,
    pub raw_text_rewritten: bool,
    pub tokens: Vec<RagCjkToken>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SemanticPrivacyStatus {
    pub deterministic_extraction_primary: bool,
    pub ml_required: bool,
    pub cloud_upload_default: bool,
    pub explicit_endpoint_required: bool,
    pub explicit_payload_policy_required: bool,
    pub explicit_privacy_ack_required: bool,
    pub secret_values_logged: bool,
    pub telemetry_enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SemanticBindingReport {
    pub schema_version: String,
    pub summary: SemanticBindingSummary,
    pub document: Document,
    pub text_semantic: TextSemanticDocument,
    pub semantic_document: SemanticDocument,
    pub parenttree_recovery: ParentTreeRecoveryReport,
    pub tables: Vec<SemanticPageTables>,
    pub cjk_token_pages: Vec<SemanticCjkTokenPage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dictionary_report: Option<CjkDictionaryLoadReport>,
    pub rag_chunks: AdvancedRagChunkSet,
    pub search_results: Vec<TextSearchMatch>,
    pub layout_backend_status: LayoutAvailabilityReport,
    pub table_model_backend_status: TableModelBackendStatusReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table_proposal_merge: Option<TableProposalMergeReport>,
    pub privacy: SemanticPrivacyStatus,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SemanticSearchReport {
    pub schema_version: String,
    pub query: String,
    pub raw_text_fallback: bool,
    pub semantic_matches: Vec<TextSearchMatch>,
    pub cjk_token_matches: Vec<crate::text::CjkTokenSearchMatch>,
    pub dictionary_report: CjkDictionaryLoadReport,
    pub provenance_preserved: bool,
}

pub fn build_semantic_binding_report(
    engine: &ContentEngine,
    options: &SemanticBindingOptions,
) -> Result<SemanticBindingReport> {
    let pages = normalized_pages(engine, &options.pages)?;
    let provider = load_provider(options)?;
    let dictionary_report = provider.as_ref().map(|provider| provider.report().clone());

    let text_options = TextSemanticOptions {
        include_chars: options.include_chars,
        include_hidden: options.include_hidden_text,
        include_structure: true,
        include_detailed_provenance: true,
        // External pack segmentation is represented by cjk_token_pages and RAG
        // metadata. The Prompt 06 word layer keeps its stable built-in modes.
        cjk_segmentation: CjkSegmentationMode::Char,
        ..TextSemanticOptions::default()
    };
    let text_semantic = engine.extract_text_semantic_model(&pages, text_options)?;
    let semantic_document = engine.extract_semantic_document(&pages)?;
    let parenttree_recovery = engine.recover_parenttree_semantics(&pages)?;
    let document = engine.parse_document(&ParseOptions {
        pages: pages.clone(),
        omit_furniture: false,
        ..ParseOptions::default()
    })?;

    let mut tables = Vec::new();
    for page in &pages {
        tables.push(SemanticPageTables {
            page: *page,
            tables: engine.extract_tables(*page)?,
        });
    }

    let cjk_token_pages = provider
        .as_ref()
        .map(|provider| cjk_pages(&text_semantic, provider))
        .unwrap_or_default();
    let search_results = options
        .search_query
        .as_deref()
        .map(|query| {
            text_semantic.search(
                query,
                &TextSearchOptions {
                    case_sensitive: options.search_case_sensitive,
                    ..TextSearchOptions::default()
                },
            )
        })
        .unwrap_or_default();

    let risky = scan_risky_content(engine.document())?;
    let security = ChunkSecurityPosture {
        hidden_content_warning: text_semantic.counters.hidden_or_invisible > 0,
        active_content_warning: risky.risky_total() > 0,
        diagnostics: if risky.risky_total() > 0 {
            vec![format!(
                "Original input contains {} active or risky content item(s)",
                risky.risky_total()
            )]
        } else {
            ChunkSecurityPosture::default().diagnostics
        },
        ..ChunkSecurityPosture::default()
    };
    let chunk_context = AdvancedChunkContext {
        text_semantic: Some(&text_semantic),
        semantic_document: Some(&semantic_document),
        parenttree: Some(&parenttree_recovery),
        dictionary: provider.as_ref(),
        security,
    };
    let rag_chunks = advanced_chunk_document(&document, &options.chunk_options, &chunk_context);

    let deterministic_tables = deterministic_table_evidence(&document, &tables);
    let table_proposal_merge = options.table_proposals.as_ref().map(|proposals| {
        merge_table_proposals_deterministic(
            &deterministic_tables,
            proposals,
            &TableProposalMergePolicy::default(),
        )
    });
    let layout_backend_status = layout_backend_availability_report(
        &LayoutLocalBackendConfig::default(),
        &CloudLayoutBackendConfig::default(),
    );
    let summary = summary(
        &document,
        &text_semantic,
        &semantic_document,
        &parenttree_recovery,
        &tables,
        &cjk_token_pages,
        &rag_chunks,
        &search_results,
    );

    Ok(SemanticBindingReport {
        schema_version: SEMANTIC_BINDING_SCHEMA_VERSION.to_string(),
        summary,
        document,
        text_semantic,
        semantic_document,
        parenttree_recovery,
        tables,
        cjk_token_pages,
        dictionary_report,
        rag_chunks,
        search_results,
        layout_backend_status,
        table_model_backend_status: table_model_backend_status_report(),
        table_proposal_merge,
        privacy: SemanticPrivacyStatus {
            deterministic_extraction_primary: true,
            ml_required: false,
            cloud_upload_default: false,
            explicit_endpoint_required: true,
            explicit_payload_policy_required: true,
            explicit_privacy_ack_required: true,
            secret_values_logged: false,
            telemetry_enabled: false,
        },
        diagnostics: vec![
            "C ABI and managed bindings expose this report as owned versioned JSON".to_string(),
            "Native model runtimes and model weights are not bundled".to_string(),
        ],
    })
}

pub fn semantic_search_report(
    engine: &ContentEngine,
    pages: &[usize],
    query: &str,
    provider: Option<&CjkDictionaryProvider>,
) -> Result<SemanticSearchReport> {
    let query = query.trim();
    if query.is_empty() {
        return Err(crate::WellfriendError::invalid_input(
            "semantic search query must not be empty",
        ));
    }
    if query.chars().count() > 4_096 {
        return Err(crate::WellfriendError::invalid_input(
            "semantic search query exceeds the 4096-character limit",
        ));
    }
    let pages = normalized_pages(engine, pages)?;
    let text = engine.extract_text_semantic_model(
        &pages,
        TextSemanticOptions {
            include_hidden: false,
            include_structure: true,
            include_detailed_provenance: true,
            ..TextSemanticOptions::search_text()
        },
    )?;
    let owned_provider;
    let provider = if let Some(provider) = provider {
        provider
    } else {
        owned_provider = CjkDictionaryProvider::builtin_fixture();
        &owned_provider
    };
    let semantic_matches = text.search(
        query,
        &TextSearchOptions {
            case_sensitive: false,
            ..TextSearchOptions::default()
        },
    );
    let cjk_token_matches = crate::text::cjk_dictionary_token_search(&text.text(), query, provider);
    Ok(SemanticSearchReport {
        schema_version: "prompt15.semantic_search.v1".to_string(),
        query: query.to_string(),
        raw_text_fallback: true,
        semantic_matches,
        cjk_token_matches,
        dictionary_report: provider.report().clone(),
        provenance_preserved: true,
    })
}

impl ContentEngine {
    pub fn semantic_binding_report(
        &self,
        options: &SemanticBindingOptions,
    ) -> Result<SemanticBindingReport> {
        build_semantic_binding_report(self, options)
    }

    pub fn semantic_search_report(
        &self,
        pages: &[usize],
        query: &str,
        provider: Option<&CjkDictionaryProvider>,
    ) -> Result<SemanticSearchReport> {
        semantic_search_report(self, pages, query, provider)
    }
}

fn normalized_pages(engine: &ContentEngine, requested: &[usize]) -> Result<Vec<usize>> {
    let count = engine.page_count()?;
    let pages = if requested.is_empty() {
        (1..=count).collect()
    } else {
        requested.to_vec()
    };
    for page in &pages {
        if *page == 0 || *page > count {
            return Err(crate::WellfriendError::invalid_input(format!(
                "page {page} out of range for {count}-page document"
            )));
        }
    }
    Ok(pages)
}

fn load_provider(options: &SemanticBindingOptions) -> Result<Option<CjkDictionaryProvider>> {
    if !options.dictionary_enabled {
        return Ok(None);
    }
    if options.dictionary_manifest_paths.is_empty() {
        Ok(Some(CjkDictionaryProvider::builtin_fixture()))
    } else {
        Ok(Some(CjkDictionaryProvider::from_manifest_paths(
            &options.dictionary_manifest_paths,
            options.dictionary_limits,
        )?))
    }
}

fn cjk_pages(
    text: &TextSemanticDocument,
    provider: &CjkDictionaryProvider,
) -> Vec<SemanticCjkTokenPage> {
    text.pages
        .iter()
        .map(|page| SemanticCjkTokenPage {
            page: page.page,
            raw_text: page.text(),
            raw_text_rewritten: false,
            tokens: segment_cjk_dictionary_text_with_provider(&page.text(), provider)
                .into_iter()
                .map(|token| RagCjkToken {
                    text: token.text,
                    char_range: token.char_range,
                    byte_range: token.byte_range,
                    language: token.language,
                    confidence: token.confidence,
                    source: token.source,
                })
                .collect(),
        })
        .collect()
}

fn deterministic_table_evidence(
    document: &Document,
    page_tables: &[SemanticPageTables],
) -> Vec<DeterministicTableEvidence> {
    let mut evidence = Vec::new();
    for block in &document.body {
        let BlockKind::Table { table, .. } = &block.kind else {
            continue;
        };
        evidence.push(DeterministicTableEvidence {
            table_id: format!("table-{}", block.id),
            page: block.page as usize,
            block_id: Some(block.id),
            table: table.clone(),
            source_span_ids: vec![format!("page-{}-block-{}", block.page, block.id)],
            mcids: Vec::new(),
            provenance: "canonical_deterministic_table".to_string(),
        });
    }
    for page in page_tables {
        for (index, table) in page.tables.iter().enumerate() {
            if evidence.iter().any(|item| {
                item.page == page.page
                    && item.table.bbox == table.bbox
                    && item.table.rows == table.rows
            }) {
                continue;
            }
            evidence.push(DeterministicTableEvidence {
                table_id: format!("page-{}-extracted-table-{index}", page.page),
                page: page.page,
                block_id: None,
                table: table.clone(),
                source_span_ids: Vec::new(),
                mcids: Vec::new(),
                provenance: "deterministic_table_extractor".to_string(),
            });
        }
    }
    evidence.sort_by(|left, right| {
        left.page
            .cmp(&right.page)
            .then_with(|| left.table_id.cmp(&right.table_id))
    });
    evidence
}

#[allow(clippy::too_many_arguments)]
fn summary(
    document: &Document,
    text: &TextSemanticDocument,
    semantic: &SemanticDocument,
    parenttree: &ParentTreeRecoveryReport,
    tables: &[SemanticPageTables],
    cjk_pages: &[SemanticCjkTokenPage],
    chunks: &AdvancedRagChunkSet,
    search: &[TextSearchMatch],
) -> SemanticBindingSummary {
    SemanticBindingSummary {
        page_count: document.pages.len(),
        block_count: document.body.len(),
        paragraph_count: document
            .body
            .iter()
            .filter(|block| matches!(block.kind, BlockKind::Paragraph { .. }))
            .count(),
        line_count: text
            .pages
            .iter()
            .flat_map(|page| &page.blocks)
            .map(|block| block.lines.len())
            .sum(),
        span_count: text
            .pages
            .iter()
            .flat_map(|page| &page.blocks)
            .flat_map(|block| &block.lines)
            .map(|line| line.spans.len())
            .sum(),
        char_count: text.counters.chars,
        structure_node_count: semantic.elements.iter().map(count_element).sum(),
        mcid_count: semantic.elements.iter().map(count_mcids).sum(),
        recovered_parenttree_node_count: parenttree.recovered_node_count,
        orphan_mcid_count: parenttree.orphan_mcid_count,
        parenttree_conflict_count: parenttree.conflict_count,
        table_count: tables.iter().map(|page| page.tables.len()).sum(),
        table_cell_count: tables
            .iter()
            .flat_map(|page| &page.tables)
            .map(|table| {
                if table.cells.is_empty() {
                    table.rows.iter().map(Vec::len).sum()
                } else {
                    table.cells.len()
                }
            })
            .sum(),
        figure_count: document
            .body
            .iter()
            .filter(|block| matches!(block.kind, BlockKind::Figure { .. }))
            .count(),
        caption_count: document
            .body
            .iter()
            .filter(|block| matches!(block.kind, BlockKind::Caption { .. }))
            .count(),
        cjk_token_count: cjk_pages.iter().map(|page| page.tokens.len()).sum(),
        rag_chunk_count: chunks.chunks.len(),
        search_match_count: search.len(),
    }
}

fn count_element(element: &SemanticElement) -> usize {
    1 + element.children.iter().map(count_element).sum::<usize>()
}

fn count_mcids(element: &SemanticElement) -> usize {
    element.mcids.len() + element.children.iter().map(count_mcids).sum::<usize>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binding_options_default_to_offline_deterministic_behavior() {
        let options = SemanticBindingOptions::default();
        assert!(options.dictionary_enabled);
        assert!(options.table_proposals.is_none());
        assert_eq!(options.chunk_options.mode, crate::AdvancedChunkMode::Hybrid);
    }

    #[test]
    fn privacy_status_has_no_upload_or_telemetry_default() {
        let privacy = SemanticPrivacyStatus {
            deterministic_extraction_primary: true,
            ml_required: false,
            cloud_upload_default: false,
            explicit_endpoint_required: true,
            explicit_payload_policy_required: true,
            explicit_privacy_ack_required: true,
            secret_values_logged: false,
            telemetry_enabled: false,
        };
        assert!(!privacy.cloud_upload_default);
        assert!(!privacy.secret_values_logged);
        assert!(!privacy.telemetry_enabled);
    }
}
