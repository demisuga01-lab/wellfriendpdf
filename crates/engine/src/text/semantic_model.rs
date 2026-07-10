use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::analysis::layout::{analyze_page, BBox, LayoutConfig, PageLayout};
use crate::error::{OxideError, Result};
use crate::fonts::FontDecodeSource;
use crate::text::{MarkedTextChunk, ReadingOrderReconstructor, TextChunk};

const PROMPT14B_CJK_PROVIDER_SCHEMA_VERSION: &str = "prompt14b.cjk_dictionary_provider.v1";
const DEFAULT_MAX_CHUNKS_PER_PAGE: usize = 250_000;
const DEFAULT_MAX_CHARS_PER_PAGE: usize = 2_000_000;
const DEFAULT_MAX_STRUCTURE_NODES: usize = 250_000;
const DEFAULT_MAX_MCID_ENTRIES: usize = 500_000;
const DEFAULT_MAX_CJK_RUN_CHARS: usize = 64;
const DEDUPE_X_TOLERANCE: f64 = 0.75;
const DEDUPE_Y_TOLERANCE: f64 = 0.75;
const DEDUPE_FONT_TOLERANCE: f64 = 0.5;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TextExtractionMode {
    ExtractAllText,
    VisibleTextOnly,
    SemanticTextPreferActual,
    SearchText,
    RedactionText,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct TextSemanticOptions {
    pub mode: TextExtractionMode,
    pub include_chars: bool,
    pub include_hidden: bool,
    pub deduplicate: bool,
    pub prefer_actual_text: bool,
    pub include_structure: bool,
    pub include_detailed_provenance: bool,
    pub cjk_segmentation: CjkSegmentationMode,
    pub max_chunks_per_page: usize,
    pub max_chars_per_page: usize,
    pub max_structure_nodes: usize,
    pub max_mcid_entries: usize,
    pub max_cjk_run_chars: usize,
}

impl Default for TextSemanticOptions {
    fn default() -> Self {
        Self {
            mode: TextExtractionMode::SemanticTextPreferActual,
            include_chars: true,
            include_hidden: true,
            deduplicate: true,
            prefer_actual_text: true,
            include_structure: true,
            include_detailed_provenance: false,
            cjk_segmentation: CjkSegmentationMode::Char,
            max_chunks_per_page: DEFAULT_MAX_CHUNKS_PER_PAGE,
            max_chars_per_page: DEFAULT_MAX_CHARS_PER_PAGE,
            max_structure_nodes: DEFAULT_MAX_STRUCTURE_NODES,
            max_mcid_entries: DEFAULT_MAX_MCID_ENTRIES,
            max_cjk_run_chars: DEFAULT_MAX_CJK_RUN_CHARS,
        }
    }
}

impl TextSemanticOptions {
    pub fn visible_text() -> Self {
        Self {
            mode: TextExtractionMode::VisibleTextOnly,
            include_hidden: false,
            ..Self::default()
        }
    }

    pub fn search_text() -> Self {
        Self {
            mode: TextExtractionMode::SearchText,
            include_hidden: false,
            ..Self::default()
        }
    }

    pub fn redaction_text() -> Self {
        Self {
            mode: TextExtractionMode::RedactionText,
            include_hidden: false,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SemanticTextDirection {
    LeftToRight,
    RightToLeft,
    Vertical,
    Mixed,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TextRole {
    BodyText,
    Heading,
    List,
    TableCandidate,
    FigureCaption,
    Header,
    Footer,
    Footnote,
    Marginalia,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TextRoleSource {
    Tagged,
    RoleMap,
    ActualText,
    Heuristic,
    Synthetic,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CjkSegmentationMode {
    Char,
    Simple,
    Dictionary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CjkDictionaryMetadata {
    pub name: String,
    pub version: String,
    pub hash: String,
    pub license: String,
    pub source: String,
    pub entry_count: usize,
    pub languages: Vec<String>,
    pub load_status: String,
    pub memory_footprint_bytes: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CjkDictionaryToken {
    pub text: String,
    pub char_range: [usize; 2],
    pub byte_range: [usize; 2],
    pub language: String,
    pub confidence: f32,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CjkDictionaryPackManifest {
    pub pack_id: String,
    pub languages: Vec<String>,
    pub scripts: Vec<String>,
    pub source: String,
    pub license: String,
    pub version: String,
    pub date: String,
    pub hash: String,
    pub entries_path: String,
    pub entry_count: usize,
    pub generation_command: String,
    pub normalization_form: String,
    pub redistribution_allowed: bool,
    pub expected_memory_footprint_bytes: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct CjkDictionaryProviderLimits {
    pub max_entries: usize,
    pub memory_cap_bytes: usize,
    pub max_token_chars: usize,
}

impl Default for CjkDictionaryProviderLimits {
    fn default() -> Self {
        Self {
            max_entries: 500_000,
            memory_cap_bytes: 64 * 1024 * 1024,
            max_token_chars: 64,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CjkDictionaryLoadDiagnostic {
    pub code: String,
    pub severity: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CjkDictionaryPackStatus {
    pub pack_id: String,
    pub load_status: String,
    pub manifest_path: String,
    pub entries_path: String,
    pub manifest: CjkDictionaryPackManifest,
    pub metadata: CjkDictionaryMetadata,
    pub duplicate_entries: usize,
    pub malformed_entries: usize,
    pub diagnostics: Vec<CjkDictionaryLoadDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CjkDictionaryLoadReport {
    pub schema_version: String,
    pub provider_status: String,
    pub limits: CjkDictionaryProviderLimits,
    pub packs: Vec<CjkDictionaryPackStatus>,
    pub total_entries: usize,
    pub duplicate_entries: usize,
    pub memory_footprint_bytes: usize,
    pub max_token_chars: usize,
    pub languages: Vec<String>,
    pub diagnostics: Vec<CjkDictionaryLoadDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CjkTokenSearchMatch {
    pub query: String,
    pub matched_text: String,
    pub token_index: usize,
    pub char_range: [usize; 2],
    pub byte_range: [usize; 2],
    pub language: String,
    pub confidence: f32,
    pub provenance: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CjkRagTokenChunk {
    pub chunk_index: usize,
    pub text: String,
    pub token_count: usize,
    pub char_range: [usize; 2],
    pub byte_range: [usize; 2],
    pub languages: Vec<String>,
    pub confidence: f32,
    pub provenance: String,
}

#[derive(Debug, Clone, Copy)]
struct CjkDictionaryEntry {
    term: &'static str,
    language: &'static str,
}

#[derive(Debug, Clone)]
struct IndexedCjkDictionaryEntry {
    term: String,
    chars: Vec<char>,
    language: String,
    priority: i32,
    source: String,
    confidence: f32,
    ordinal: usize,
}

#[derive(Debug, Clone)]
pub struct CjkDictionaryProvider {
    entries: Vec<IndexedCjkDictionaryEntry>,
    metadata: Vec<CjkDictionaryMetadata>,
    report: CjkDictionaryLoadReport,
    max_token_chars: usize,
}

const BUILTIN_CJK_DICTIONARY_NAME: &str = "oxide-prompt14-synthetic-cjk-test-dictionary";
const BUILTIN_CJK_DICTIONARY_VERSION: &str = "2026-07-09";
const BUILTIN_CJK_DICTIONARY_LICENSE: &str = "CC0-1.0 synthetic fixture terms";

const BUILTIN_CJK_DICTIONARY: &[CjkDictionaryEntry] = &[
    CjkDictionaryEntry {
        term: "\u{4EBA}\u{5DE5}\u{667A}\u{80FD}",
        language: "zh",
    },
    CjkDictionaryEntry {
        term: "\u{673A}\u{5668}\u{5B66}\u{4E60}",
        language: "zh",
    },
    CjkDictionaryEntry {
        term: "\u{6570}\u{636E}\u{5E93}",
        language: "zh",
    },
    CjkDictionaryEntry {
        term: "\u{5317}\u{4EAC}\u{5927}\u{5B66}",
        language: "zh",
    },
    CjkDictionaryEntry {
        term: "\u{4E1C}\u{4EAC}\u{5927}\u{5B66}",
        language: "ja",
    },
    CjkDictionaryEntry {
        term: "\u{5F62}\u{614B}\u{7D20}\u{89E3}\u{6790}",
        language: "ja",
    },
    CjkDictionaryEntry {
        term: "\u{691C}\u{7D22}\u{30A8}\u{30F3}\u{30B8}\u{30F3}",
        language: "ja",
    },
    CjkDictionaryEntry {
        term: "\u{D55C}\u{AD6D}\u{C5B4}",
        language: "ko",
    },
    CjkDictionaryEntry {
        term: "\u{C790}\u{C5F0}\u{C5B4}\u{CC98}\u{B9AC}",
        language: "ko",
    },
    CjkDictionaryEntry {
        term: "\u{AC80}\u{C0C9}\u{C5D4}\u{C9C4}",
        language: "ko",
    },
];

impl CjkDictionaryProvider {
    pub fn builtin_fixture() -> Self {
        let limits = CjkDictionaryProviderLimits::default();
        let entries = BUILTIN_CJK_DICTIONARY
            .iter()
            .enumerate()
            .map(|(ordinal, entry)| IndexedCjkDictionaryEntry {
                term: entry.term.to_string(),
                chars: entry.term.chars().collect(),
                language: entry.language.to_string(),
                priority: 0,
                source: "builtin_fixture".to_string(),
                confidence: 0.96,
                ordinal,
            })
            .collect::<Vec<_>>();
        let metadata = builtin_cjk_dictionary_metadata();
        let memory_footprint_bytes = metadata.memory_footprint_bytes;
        let report = CjkDictionaryLoadReport {
            schema_version: PROMPT14B_CJK_PROVIDER_SCHEMA_VERSION.to_string(),
            provider_status: "loaded_builtin_fixture".to_string(),
            limits,
            packs: vec![CjkDictionaryPackStatus {
                pack_id: metadata.name.clone(),
                load_status: metadata.load_status.clone(),
                manifest_path: "builtin".to_string(),
                entries_path: "builtin".to_string(),
                manifest: CjkDictionaryPackManifest {
                    pack_id: metadata.name.clone(),
                    languages: metadata.languages.clone(),
                    scripts: vec!["Han".to_string(), "Kana".to_string(), "Hangul".to_string()],
                    source: metadata.source.clone(),
                    license: metadata.license.clone(),
                    version: metadata.version.clone(),
                    date: metadata.version.clone(),
                    hash: metadata.hash.clone(),
                    entries_path: "builtin".to_string(),
                    entry_count: metadata.entry_count,
                    generation_command: "compiled synthetic fixture".to_string(),
                    normalization_form: "trim_no_unicode_rewrite".to_string(),
                    redistribution_allowed: true,
                    expected_memory_footprint_bytes: metadata.memory_footprint_bytes,
                },
                metadata: metadata.clone(),
                duplicate_entries: 0,
                malformed_entries: 0,
                diagnostics: Vec::new(),
            }],
            total_entries: entries.len(),
            duplicate_entries: 0,
            memory_footprint_bytes,
            max_token_chars: limits.max_token_chars,
            languages: metadata.languages.clone(),
            diagnostics: Vec::new(),
        };
        Self {
            entries,
            metadata: vec![metadata],
            report,
            max_token_chars: limits.max_token_chars,
        }
    }

    pub fn from_manifest_paths(
        manifest_paths: &[PathBuf],
        limits: CjkDictionaryProviderLimits,
    ) -> Result<Self> {
        if manifest_paths.is_empty() {
            return Err(OxideError::invalid_input(
                "at least one CJK dictionary manifest path is required",
            ));
        }

        let mut raw_entries = Vec::new();
        let mut packs = Vec::new();
        let mut diagnostics = Vec::new();
        let mut memory_footprint_bytes = 0usize;

        for manifest_path in manifest_paths {
            let manifest_bytes = fs::read(manifest_path)?;
            let manifest: CjkDictionaryPackManifest = serde_json::from_slice(&manifest_bytes)
                .map_err(|err| {
                    OxideError::invalid_input(format!(
                        "invalid CJK dictionary manifest {}: {err}",
                        manifest_path.display()
                    ))
                })?;
            if manifest.entry_count > limits.max_entries {
                return Err(OxideError::ResourceLimit(format!(
                    "CJK dictionary pack {} declares {} entries above cap {}",
                    manifest.pack_id, manifest.entry_count, limits.max_entries
                )));
            }
            if manifest.expected_memory_footprint_bytes > limits.memory_cap_bytes {
                return Err(OxideError::ResourceLimit(format!(
                    "CJK dictionary pack {} declares {} bytes above cap {}",
                    manifest.pack_id,
                    manifest.expected_memory_footprint_bytes,
                    limits.memory_cap_bytes
                )));
            }

            let entries_path = resolve_manifest_entries_path(manifest_path, &manifest.entries_path);
            let entries_bytes = fs::read(&entries_path)?;
            let actual_hash = sha256_digest(&entries_bytes);
            if !manifest.hash.is_empty() && manifest.hash != actual_hash {
                return Err(OxideError::invalid_input(format!(
                    "CJK dictionary pack {} hash mismatch: expected {}, got {}",
                    manifest.pack_id, manifest.hash, actual_hash
                )));
            }

            let mut pack_malformed = 0usize;
            let mut pack_entries = parse_dictionary_tsv_entries(
                &manifest,
                &entries_bytes,
                limits.max_token_chars,
                &mut pack_malformed,
            )?;
            if pack_malformed > 0 {
                return Err(OxideError::invalid_input(format!(
                    "CJK dictionary pack {} contains {} malformed TSV entries",
                    manifest.pack_id, pack_malformed
                )));
            }
            memory_footprint_bytes += pack_entries
                .iter()
                .map(|entry| entry.term.len() + entry.language.len() + entry.source.len() + 16)
                .sum::<usize>();
            if memory_footprint_bytes > limits.memory_cap_bytes {
                return Err(OxideError::ResourceLimit(format!(
                    "CJK dictionary memory estimate {} exceeded cap {}",
                    memory_footprint_bytes, limits.memory_cap_bytes
                )));
            }
            raw_entries.append(&mut pack_entries);
            let metadata = CjkDictionaryMetadata {
                name: manifest.pack_id.clone(),
                version: manifest.version.clone(),
                hash: actual_hash,
                license: manifest.license.clone(),
                source: manifest.source.clone(),
                entry_count: manifest.entry_count,
                languages: manifest.languages.clone(),
                load_status: "loaded_external_pack".to_string(),
                memory_footprint_bytes: manifest.expected_memory_footprint_bytes,
            };
            packs.push(CjkDictionaryPackStatus {
                pack_id: manifest.pack_id.clone(),
                load_status: "loaded_external_pack".to_string(),
                manifest_path: manifest_path.display().to_string(),
                entries_path: entries_path.display().to_string(),
                manifest,
                metadata,
                duplicate_entries: 0,
                malformed_entries: pack_malformed,
                diagnostics: Vec::new(),
            });
        }

        if raw_entries.len() > limits.max_entries {
            return Err(OxideError::ResourceLimit(format!(
                "CJK dictionary loaded {} entries above cap {}",
                raw_entries.len(),
                limits.max_entries
            )));
        }

        let (entries, duplicate_entries) = dedupe_and_order_dictionary_entries(raw_entries);
        if entries.is_empty() {
            return Err(OxideError::invalid_input(
                "CJK dictionary provider loaded no valid entries",
            ));
        }
        if duplicate_entries > 0 {
            diagnostics.push(CjkDictionaryLoadDiagnostic {
                code: "dictionary_provider.duplicate_entries_deduped".to_string(),
                severity: "info".to_string(),
                message: format!(
                    "{duplicate_entries} duplicate entries were deterministically deduplicated"
                ),
                path: None,
            });
        }
        for pack in &mut packs {
            pack.duplicate_entries = duplicate_entries;
        }

        let languages = entries
            .iter()
            .map(|entry| entry.language.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let metadata = packs
            .iter()
            .map(|pack| pack.metadata.clone())
            .collect::<Vec<_>>();
        let report = CjkDictionaryLoadReport {
            schema_version: PROMPT14B_CJK_PROVIDER_SCHEMA_VERSION.to_string(),
            provider_status: "loaded_external_packs".to_string(),
            limits,
            packs,
            total_entries: entries.len(),
            duplicate_entries,
            memory_footprint_bytes,
            max_token_chars: limits.max_token_chars,
            languages,
            diagnostics,
        };
        Ok(Self {
            entries,
            metadata,
            report,
            max_token_chars: limits.max_token_chars,
        })
    }

    pub fn report(&self) -> &CjkDictionaryLoadReport {
        &self.report
    }

    pub fn metadata(&self) -> &[CjkDictionaryMetadata] {
        &self.metadata
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    fn best_match(&self, chars: &[char], start: usize) -> Option<&IndexedCjkDictionaryEntry> {
        let mut best: Option<&IndexedCjkDictionaryEntry> = None;
        for entry in &self.entries {
            let len = entry.chars.len();
            if len == 0
                || len > self.max_token_chars
                || start + len > chars.len()
                || !chars[start..start + len]
                    .iter()
                    .copied()
                    .eq(entry.chars.iter().copied())
            {
                continue;
            }
            if best.is_none_or(|active| {
                len > active.chars.len()
                    || (len == active.chars.len()
                        && (entry.priority > active.priority
                            || (entry.priority == active.priority
                                && entry.ordinal < active.ordinal)))
            }) {
                best = Some(entry);
            }
        }
        best
    }
}

pub fn builtin_cjk_dictionary_metadata() -> CjkDictionaryMetadata {
    let memory_footprint_bytes = BUILTIN_CJK_DICTIONARY
        .iter()
        .map(|entry| entry.term.len() + entry.language.len())
        .sum();
    CjkDictionaryMetadata {
        name: BUILTIN_CJK_DICTIONARY_NAME.to_string(),
        version: BUILTIN_CJK_DICTIONARY_VERSION.to_string(),
        hash: builtin_cjk_dictionary_hash(),
        license: BUILTIN_CJK_DICTIONARY_LICENSE.to_string(),
        source: "compiled_synthetic_test_fixture".to_string(),
        entry_count: BUILTIN_CJK_DICTIONARY.len(),
        languages: vec!["zh".to_string(), "ja".to_string(), "ko".to_string()],
        load_status: "loaded_builtin".to_string(),
        memory_footprint_bytes,
    }
}

pub fn segment_cjk_dictionary_text(text: &str) -> Vec<CjkDictionaryToken> {
    let provider = CjkDictionaryProvider::builtin_fixture();
    segment_cjk_dictionary_text_with_provider(text, &provider)
}

pub fn segment_cjk_dictionary_text_with_provider(
    text: &str,
    provider: &CjkDictionaryProvider,
) -> Vec<CjkDictionaryToken> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let cjk_chars = chars.iter().map(|(_, c)| *c).collect::<Vec<_>>();
    let mut out = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let (_, c) = chars[index];
        if c.is_whitespace() {
            index += 1;
            continue;
        }
        if is_cjk_punctuation(c) {
            let start_index = index;
            let start_byte = chars[index].0;
            index += 1;
            let end_byte = chars
                .get(index)
                .map(|(byte, _)| *byte)
                .unwrap_or_else(|| text.len());
            out.push(CjkDictionaryToken {
                text: text[start_byte..end_byte].to_string(),
                char_range: [start_index, index],
                byte_range: [start_byte, end_byte],
                language: "punctuation".to_string(),
                confidence: 1.0,
                source: "script_boundary".to_string(),
            });
            continue;
        }
        if !is_cjk_char(c) {
            let start_index = index;
            let start_byte = chars[index].0;
            while index < chars.len() {
                let (_, active) = chars[index];
                if active.is_whitespace() || is_cjk_char(active) || is_cjk_punctuation(active) {
                    break;
                }
                index += 1;
            }
            let end_byte = chars
                .get(index)
                .map(|(byte, _)| *byte)
                .unwrap_or_else(|| text.len());
            out.push(CjkDictionaryToken {
                text: text[start_byte..end_byte].to_string(),
                char_range: [start_index, index],
                byte_range: [start_byte, end_byte],
                language: "mixed_latin".to_string(),
                confidence: 0.74,
                source: "script_boundary".to_string(),
            });
            continue;
        }
        let start_index = index;
        let start_byte = chars[index].0;
        let best = provider.best_match(&cjk_chars, index);
        let (len, language, confidence, source) = if let Some(entry) = best {
            (
                entry.chars.len(),
                entry.language.as_str(),
                entry.confidence,
                entry.source.as_str(),
            )
        } else {
            (1, language_for_cjk_char(c), 0.42, "unknown_cjk_fallback")
        };
        index += len;
        let end_byte = chars
            .get(index)
            .map(|(byte, _)| *byte)
            .unwrap_or_else(|| text.len());
        out.push(CjkDictionaryToken {
            text: text[start_byte..end_byte].to_string(),
            char_range: [start_index, index],
            byte_range: [start_byte, end_byte],
            language: language.to_string(),
            confidence,
            source: source.to_string(),
        });
    }
    out
}

pub fn cjk_dictionary_token_search(
    text: &str,
    query: &str,
    provider: &CjkDictionaryProvider,
) -> Vec<CjkTokenSearchMatch> {
    let tokens = segment_cjk_dictionary_text_with_provider(text, provider);
    let query_tokens = segment_cjk_dictionary_text_with_provider(query, provider);
    if query_tokens.is_empty() {
        return Vec::new();
    }
    let query_texts = query_tokens
        .iter()
        .map(|token| token.text.as_str())
        .collect::<Vec<_>>();
    let mut matches = Vec::new();
    for start in 0..tokens.len() {
        if start + query_texts.len() > tokens.len() {
            break;
        }
        if tokens[start..start + query_texts.len()]
            .iter()
            .map(|token| token.text.as_str())
            .eq(query_texts.iter().copied())
        {
            let end = start + query_texts.len() - 1;
            let confidence = tokens[start..=end]
                .iter()
                .map(|token| token.confidence)
                .fold(1.0f32, f32::min);
            matches.push(CjkTokenSearchMatch {
                query: query.to_string(),
                matched_text: tokens[start..=end]
                    .iter()
                    .map(|token| token.text.as_str())
                    .collect::<Vec<_>>()
                    .join(""),
                token_index: start,
                char_range: [tokens[start].char_range[0], tokens[end].char_range[1]],
                byte_range: [tokens[start].byte_range[0], tokens[end].byte_range[1]],
                language: tokens[start].language.clone(),
                confidence,
                provenance: "dictionary_token_layer".to_string(),
            });
        }
    }
    matches
}

pub fn cjk_dictionary_rag_token_chunks(
    text: &str,
    provider: &CjkDictionaryProvider,
    max_tokens_per_chunk: usize,
) -> Vec<CjkRagTokenChunk> {
    let tokens = segment_cjk_dictionary_text_with_provider(text, provider);
    let chunk_size = max_tokens_per_chunk.max(1);
    let mut chunks = Vec::new();
    for slice in tokens.chunks(chunk_size) {
        let Some(first) = slice.first() else {
            continue;
        };
        let last = slice.last().unwrap_or(first);
        let languages = slice
            .iter()
            .map(|token| token.language.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let confidence = slice
            .iter()
            .map(|token| token.confidence)
            .fold(1.0f32, f32::min);
        chunks.push(CjkRagTokenChunk {
            chunk_index: chunks.len(),
            text: slice
                .iter()
                .map(|token| token.text.as_str())
                .collect::<Vec<_>>()
                .join(""),
            token_count: slice.len(),
            char_range: [first.char_range[0], last.char_range[1]],
            byte_range: [first.byte_range[0], last.byte_range[1]],
            languages,
            confidence,
            provenance: "dictionary_token_layer_preserves_source_offsets".to_string(),
        });
    }
    chunks
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TextMappingSource {
    NativePdfText,
    TaggedPdf,
    ActualText,
    ToUnicode,
    EmbeddedCMap,
    PredefinedCMap,
    EncodingDifferences,
    GlyphName,
    UniName,
    FontCMap,
    IdentityCid,
    Ocr,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TextProvenanceFlag {
    NativePdfText,
    TaggedPdf,
    TaggedMcid,
    StructTreeRole,
    ActualText,
    ToUnicode,
    Ocr,
    FallbackCMap,
    PredefinedCMap,
    FallbackGlyphName,
    EncodingDifferences,
    FontCMap,
    IdentityCid,
    LigatureExpansion,
    HyphenationJoin,
    NormalizedWhitespace,
    SyntheticLayout,
    HeuristicRole,
    LowConfidenceOrder,
    Deduplicated,
    HiddenOrInvisible,
    ArtifactHeaderFooterCandidate,
    DictionarySegmented,
    UnknownUnmapped,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct TextProvenanceSummary {
    pub actual_text: usize,
    pub tounicode: usize,
    pub embedded_cmap: usize,
    pub predefined_cmap: usize,
    pub encoding_differences: usize,
    pub glyph_name: usize,
    pub font_cmap: usize,
    pub identity_cid: usize,
    pub ocr: usize,
    pub synthetic_layout: usize,
    pub hidden_or_invisible: usize,
    pub unknown_unmapped: usize,
    pub tagged_mcid: usize,
    pub heuristic_role: usize,
    pub ligature_expansion: usize,
    pub hyphenation_join: usize,
    pub normalized_whitespace: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct TextStructureEntry {
    pub page: usize,
    pub mcid: i64,
    pub role: TextRole,
    pub normalized_role: String,
    pub original_role: String,
    pub role_source: TextRoleSource,
    pub confidence: f32,
    pub artifact: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub struct_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct TextStructureContext {
    pub entries: Vec<TextStructureEntry>,
    pub diagnostics: Vec<TextDiagnostic>,
    pub capped: bool,
}

impl TextStructureContext {
    pub fn empty() -> Self {
        Self::default()
    }

    fn by_mcid(&self) -> HashMap<(usize, i64), TextStructureEntry> {
        let mut map = HashMap::with_capacity(self.entries.len());
        for entry in &self.entries {
            map.entry((entry.page, entry.mcid))
                .or_insert_with(|| entry.clone());
        }
        map
    }

    pub fn page_summary(
        &self,
        page: usize,
        mapped_mcids: usize,
        unmapped_mcids: usize,
    ) -> TextStructurePageSummary {
        TextStructurePageSummary {
            enabled: !self.entries.is_empty() || !self.diagnostics.is_empty(),
            entries: self
                .entries
                .iter()
                .filter(|entry| entry.page == page)
                .count(),
            mapped_mcids,
            unmapped_mcids,
            capped: self.capped,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TextLayoutStrategy {
    TaggedPdf,
    XyCutGeometry,
    VerticalWriting,
    VisualFallback,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TextDiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct TextDiagnostic {
    pub code: String,
    pub severity: TextDiagnosticSeverity,
    pub page: Option<usize>,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct TextQuad {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl TextQuad {
    pub fn from_bbox(bbox: [f64; 4]) -> Self {
        Self {
            x0: bbox[0].min(bbox[2]),
            y0: bbox[1].min(bbox[3]),
            x1: bbox[0].max(bbox[2]),
            y1: bbox[1].max(bbox[3]),
        }
    }

    pub fn union(quads: &[TextQuad]) -> Option<Self> {
        let first = *quads.first()?;
        Some(quads.iter().skip(1).fold(first, |acc, q| Self {
            x0: acc.x0.min(q.x0),
            y0: acc.y0.min(q.y0),
            x1: acc.x1.max(q.x1),
            y1: acc.y1.max(q.y1),
        }))
    }

    pub fn intersects_bbox(self, bbox: BBox) -> bool {
        self.x0 < bbox.x1 && self.x1 > bbox.x0 && self.y0 < bbox.y1 && self.y1 > bbox.y0
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TextSemanticChar {
    pub text: String,
    pub unicode: String,
    pub char_index: usize,
    pub chunk_index: usize,
    pub font_name: String,
    pub font_size: f64,
    pub direction: SemanticTextDirection,
    pub mapping_source: TextMappingSource,
    pub provenance: Vec<TextProvenanceFlag>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcid: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub struct_role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_role: Option<String>,
    pub role_source: TextRoleSource,
    pub quad: TextQuad,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct TextSemanticWord {
    pub text: String,
    pub word_index: usize,
    pub char_range: [usize; 2],
    pub quad: TextQuad,
    pub confidence: f32,
    pub provenance: Vec<TextProvenanceFlag>,
    pub provenance_summary: TextProvenanceSummary,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TextSemanticSpan {
    pub text: String,
    pub span_index: usize,
    pub char_range: [usize; 2],
    pub quad: TextQuad,
    pub font_name: String,
    pub font_size: f64,
    pub direction: SemanticTextDirection,
    pub mapping_source: TextMappingSource,
    pub provenance: Vec<TextProvenanceFlag>,
    pub provenance_summary: TextProvenanceSummary,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcids: Vec<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub struct_role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_role: Option<String>,
    pub role_source: TextRoleSource,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct TextSemanticLine {
    pub text: String,
    pub line_index: usize,
    pub role: TextRole,
    pub direction: SemanticTextDirection,
    pub words: Vec<TextSemanticWord>,
    pub spans: Vec<TextSemanticSpan>,
    pub chars: Vec<TextSemanticChar>,
    pub quad: TextQuad,
    pub confidence: f32,
    pub provenance: Vec<TextProvenanceFlag>,
    pub provenance_summary: TextProvenanceSummary,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcids: Vec<i64>,
    pub role_source: TextRoleSource,
    pub role_confidence: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct TextSemanticParagraph {
    pub text: String,
    pub paragraph_index: usize,
    pub line_range: [usize; 2],
    pub role: TextRole,
    pub quad: TextQuad,
    pub confidence: f32,
    pub role_source: TextRoleSource,
    pub role_confidence: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct TextSemanticBlock {
    pub text: String,
    pub block_index: usize,
    pub role: TextRole,
    pub lines: Vec<TextSemanticLine>,
    pub paragraphs: Vec<TextSemanticParagraph>,
    pub quad: TextQuad,
    pub confidence: f32,
    pub provenance: Vec<TextProvenanceFlag>,
    pub provenance_summary: TextProvenanceSummary,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcids: Vec<i64>,
    pub role_source: TextRoleSource,
    pub role_confidence: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub struct_role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_role: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct TextExtractionCounters {
    pub pages: usize,
    pub blocks: usize,
    pub lines: usize,
    pub words: usize,
    pub chars: usize,
    pub total_glyph_runs: usize,
    pub mapped_via_tounicode: usize,
    pub mapped_via_actual_text: usize,
    pub mapped_via_cmap: usize,
    pub mapped_via_encoding_differences: usize,
    pub mapped_via_glyph_name: usize,
    pub mapped_via_ocr: usize,
    pub unknown_unmapped: usize,
    pub hidden_or_invisible: usize,
    pub rtl_runs: usize,
    pub vertical_runs: usize,
    pub deduplicated_runs: usize,
    pub low_confidence_order_edges: usize,
    pub struct_tree_nodes: usize,
    pub mcids_mapped: usize,
    pub mcids_unmapped: usize,
    pub cjk_tokens: usize,
    pub cjk_simple_tokens: usize,
    pub cjk_dictionary_tokens: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct TextSemanticPage {
    pub page: usize,
    pub page_box: [f64; 4],
    pub blocks: Vec<TextSemanticBlock>,
    pub strategy: TextLayoutStrategy,
    pub confidence: f32,
    pub counters: TextExtractionCounters,
    pub diagnostics: Vec<TextDiagnostic>,
    pub structure: TextStructurePageSummary,
}

impl TextSemanticPage {
    pub fn text(&self) -> String {
        self.blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TextSemanticDocument {
    pub pages: Vec<TextSemanticPage>,
    pub counters: TextExtractionCounters,
    pub diagnostics: Vec<TextDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct TextStructurePageSummary {
    pub enabled: bool,
    pub entries: usize,
    pub mapped_mcids: usize,
    pub unmapped_mcids: usize,
    pub capped: bool,
}

impl TextSemanticDocument {
    pub fn text(&self) -> String {
        self.pages
            .iter()
            .map(TextSemanticPage::text)
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    pub fn search(&self, query: &str, options: &TextSearchOptions) -> Vec<TextSearchMatch> {
        search_semantic_document(self, query, options)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TextSearchOptions {
    pub case_sensitive: bool,
    pub normalize_ligatures: bool,
    pub ignore_hyphenation: bool,
    pub collapse_whitespace: bool,
    pub include_hidden: bool,
    pub max_matches: usize,
}

impl Default for TextSearchOptions {
    fn default() -> Self {
        Self {
            case_sensitive: true,
            normalize_ligatures: true,
            ignore_hyphenation: true,
            collapse_whitespace: true,
            include_hidden: false,
            max_matches: 10_000,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TextSearchMatch {
    pub page: usize,
    pub text: String,
    pub normalized_text: String,
    pub char_range: [usize; 2],
    pub quads: Vec<TextQuad>,
    pub confidence: f32,
    pub provenance: Vec<TextProvenanceFlag>,
    pub provenance_summary: TextProvenanceSummary,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcids: Vec<i64>,
    pub role: TextRole,
    pub role_source: TextRoleSource,
    pub includes_hidden: bool,
}

#[derive(Debug, Clone)]
struct ChunkRef {
    chunk: TextChunk,
    original_index: usize,
    bbox: TextQuad,
    mcid: Option<i64>,
    structure: Option<TextStructureEntry>,
}

#[derive(Debug, Clone)]
struct BuiltLine {
    text: String,
    direction: SemanticTextDirection,
    words: Vec<TextSemanticWord>,
    spans: Vec<TextSemanticSpan>,
    chars: Vec<TextSemanticChar>,
    quad: TextQuad,
    provenance: Vec<TextProvenanceFlag>,
}

pub fn build_text_semantic_page(
    page: usize,
    page_box: [f64; 4],
    chunks: Vec<TextChunk>,
    options: &TextSemanticOptions,
) -> TextSemanticPage {
    let marked = chunks
        .into_iter()
        .map(|chunk| MarkedTextChunk { chunk, mcid: None })
        .collect();
    build_text_semantic_page_from_marked_chunks(page, page_box, marked, None, options)
}

pub fn build_text_semantic_page_from_marked_chunks(
    page: usize,
    page_box: [f64; 4],
    chunks: Vec<MarkedTextChunk>,
    structure: Option<&TextStructureContext>,
    options: &TextSemanticOptions,
) -> TextSemanticPage {
    let mut diagnostics = Vec::new();
    let mut counters = TextExtractionCounters {
        pages: 1,
        total_glyph_runs: chunks.len(),
        ..Default::default()
    };
    if let Some(ctx) = structure {
        diagnostics.extend(
            ctx.diagnostics
                .iter()
                .filter(|diag| diag.page == Some(page) || diag.page.is_none())
                .cloned(),
        );
    }

    let structure_map = structure
        .map(TextStructureContext::by_mcid)
        .unwrap_or_default();
    let mut working = filter_chunks(
        page,
        chunks,
        &structure_map,
        options,
        &mut counters,
        &mut diagnostics,
    );
    if working.len() > options.max_chunks_per_page {
        diagnostics.push(TextDiagnostic {
            code: "text.semantic.chunk_cap".to_string(),
            severity: TextDiagnosticSeverity::Warning,
            page: Some(page),
            message: format!(
                "page has {} text runs; semantic model capped at {}",
                working.len(),
                options.max_chunks_per_page
            ),
        });
        working.truncate(options.max_chunks_per_page);
    }

    let layout_chunks: Vec<TextChunk> = working.iter().map(|r| r.chunk.clone()).collect();
    let layout = analyze_page(&layout_chunks, &LayoutConfig::default());
    let mut block_specs = layout_to_block_specs(&layout);
    append_vertical_block_specs(&mut block_specs, &layout_chunks);
    if block_specs.is_empty() && !layout_chunks.is_empty() {
        block_specs.push(fallback_block_spec(&layout_chunks));
    }

    let mut used_chars = 0usize;
    let median_font_size = median_font_size(&layout_chunks).unwrap_or(12.0);
    let page_height = (page_box[3] - page_box[1]).abs().max(1.0);
    let mut blocks = Vec::new();
    let mut global_char_index = 0usize;
    let mut global_word_index = 0usize;
    let mut global_span_index = 0usize;
    let mut line_index = 0usize;

    for (block_index, spec) in block_specs.into_iter().enumerate() {
        let role = classify_block(
            spec.bbox,
            spec.font_size,
            median_font_size,
            page_box,
            page_height,
        );
        let mut lines = Vec::new();
        for line in spec.lines {
            let candidates = chunks_for_bbox(&working, line.bbox, line.direction);
            let built = if candidates.is_empty() {
                build_line_from_text(
                    &line.text,
                    line.bbox,
                    line.direction,
                    line_index,
                    &mut global_char_index,
                    &mut global_word_index,
                    &mut global_span_index,
                    options,
                )
            } else {
                build_line_from_chunks(
                    &candidates,
                    line.bbox,
                    line.direction,
                    line_index,
                    &mut global_char_index,
                    &mut global_word_index,
                    &mut global_span_index,
                    options,
                )
            };
            used_chars += built.chars.len();
            if used_chars > options.max_chars_per_page {
                diagnostics.push(TextDiagnostic {
                    code: "text.semantic.char_cap".to_string(),
                    severity: TextDiagnosticSeverity::Warning,
                    page: Some(page),
                    message: format!(
                        "page semantic characters exceeded cap {}; remaining text omitted",
                        options.max_chars_per_page
                    ),
                });
                break;
            }
            let cjk_tokens = built
                .words
                .iter()
                .filter(|word| word.text.chars().any(is_cjk_char))
                .count();
            counters.cjk_tokens += cjk_tokens;
            match options.cjk_segmentation {
                CjkSegmentationMode::Char => {}
                CjkSegmentationMode::Simple => counters.cjk_simple_tokens += cjk_tokens,
                CjkSegmentationMode::Dictionary => counters.cjk_dictionary_tokens += cjk_tokens,
            }
            counters.words += built.words.len();
            counters.chars += built.chars.len();
            let heuristic_role =
                classify_line(&built.text, role, built.quad, page_box, median_font_size);
            let (line_role, role_source, role_confidence) = role_from_spans(&built.spans)
                .unwrap_or((heuristic_role, TextRoleSource::Heuristic, 0.64));
            let line_summary = provenance_summary_for_spans(&built.spans);
            let line_mcids = mcids_for_spans(&built.spans);
            counters.mapped_via_tounicode += line_summary.tounicode;
            counters.mapped_via_cmap += line_summary.embedded_cmap
                + line_summary.predefined_cmap
                + line_summary.identity_cid;
            counters.mapped_via_encoding_differences += line_summary.encoding_differences;
            counters.mapped_via_glyph_name += line_summary.glyph_name;
            counters.mapped_via_ocr += line_summary.ocr;
            counters.unknown_unmapped += line_summary.unknown_unmapped;
            lines.push(TextSemanticLine {
                text: built.text,
                line_index,
                role: line_role,
                direction: built.direction,
                words: built.words,
                spans: built.spans,
                chars: built.chars,
                quad: built.quad,
                confidence: if built
                    .provenance
                    .contains(&TextProvenanceFlag::SyntheticLayout)
                {
                    0.74
                } else {
                    0.86
                },
                provenance: built.provenance,
                provenance_summary: line_summary,
                mcids: line_mcids,
                role_source,
                role_confidence,
            });
            line_index += 1;
        }
        if lines.is_empty() {
            continue;
        }
        let quad = TextQuad::union(
            &lines
                .iter()
                .map(|line| line.quad)
                .collect::<Vec<TextQuad>>(),
        )
        .unwrap_or_else(|| {
            TextQuad::from_bbox([spec.bbox.x0, spec.bbox.y0, spec.bbox.x1, spec.bbox.y1])
        });
        let text = lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let (block_role, block_role_source, block_role_confidence) =
            role_from_lines(&lines).unwrap_or((role, TextRoleSource::Heuristic, 0.58));
        let paragraphs = build_paragraphs(&lines, block_role);
        counters.lines += lines.len();
        counters.blocks += 1;
        let block_summary = provenance_summary_for_lines(&lines);
        let block_mcids = mcids_for_lines(&lines);
        let (struct_role, original_role) = block_structure_roles(&lines);
        blocks.push(TextSemanticBlock {
            text,
            block_index,
            role: block_role,
            lines,
            paragraphs,
            quad,
            confidence: if matches!(block_role, TextRole::Unknown) {
                0.58
            } else {
                0.78
            },
            provenance: vec![TextProvenanceFlag::SyntheticLayout],
            provenance_summary: block_summary,
            mcids: block_mcids,
            role_source: block_role_source,
            role_confidence: block_role_confidence,
            struct_role,
            original_role,
        });
    }

    let strategy = if counters.vertical_runs > 0 && layout.blocks.is_empty() {
        TextLayoutStrategy::VerticalWriting
    } else if !layout.blocks.is_empty() {
        TextLayoutStrategy::XyCutGeometry
    } else {
        TextLayoutStrategy::VisualFallback
    };
    if matches!(strategy, TextLayoutStrategy::VisualFallback) && !working.is_empty() {
        counters.low_confidence_order_edges += 1;
        diagnostics.push(TextDiagnostic {
            code: "text.layout.low_confidence_order".to_string(),
            severity: TextDiagnosticSeverity::Warning,
            page: Some(page),
            message: "semantic model used fallback visual ordering".to_string(),
        });
    }
    if counters.hidden_or_invisible > 0 {
        diagnostics.push(TextDiagnostic {
            code: "text.visibility.hidden_or_invisible".to_string(),
            severity: TextDiagnosticSeverity::Info,
            page: Some(page),
            message: format!(
                "{} hidden or invisible text runs observed",
                counters.hidden_or_invisible
            ),
        });
    }
    if counters.mapped_via_actual_text > 0 {
        diagnostics.push(TextDiagnostic {
            code: "text.actual_text.used".to_string(),
            severity: TextDiagnosticSeverity::Info,
            page: Some(page),
            message: format!(
                "{} characters came from ActualText replacement",
                counters.mapped_via_actual_text
            ),
        });
    }

    let structure_summary = structure
        .map(|ctx| ctx.page_summary(page, counters.mcids_mapped, counters.mcids_unmapped))
        .unwrap_or_default();

    TextSemanticPage {
        page,
        page_box,
        blocks,
        strategy,
        confidence: if counters.low_confidence_order_edges > 0 {
            0.68
        } else {
            0.84
        },
        counters,
        diagnostics,
        structure: structure_summary,
    }
}

pub fn build_text_semantic_document(
    pages: Vec<TextSemanticPage>,
    mut diagnostics: Vec<TextDiagnostic>,
) -> TextSemanticDocument {
    let mut counters = TextExtractionCounters::default();
    for page in &pages {
        merge_counters(&mut counters, &page.counters);
        diagnostics.extend(page.diagnostics.clone());
    }
    TextSemanticDocument {
        pages,
        counters,
        diagnostics,
    }
}

fn filter_chunks(
    page: usize,
    chunks: Vec<MarkedTextChunk>,
    structure_map: &HashMap<(usize, i64), TextStructureEntry>,
    options: &TextSemanticOptions,
    counters: &mut TextExtractionCounters,
    diagnostics: &mut Vec<TextDiagnostic>,
) -> Vec<ChunkRef> {
    let mut out = Vec::new();
    let mut mapped_mcids = HashSet::new();
    let mut unmapped_mcids = HashSet::new();
    for (idx, marked) in chunks.into_iter().enumerate() {
        let chunk = marked.chunk;
        if chunk.text.is_empty() {
            continue;
        }
        if chunk.is_invisible {
            counters.hidden_or_invisible += 1;
        }
        if chunk.is_rtl {
            counters.rtl_runs += 1;
        }
        if chunk.is_vertical {
            counters.vertical_runs += 1;
        }
        if chunk.is_actual_text {
            counters.mapped_via_actual_text += chunk.text.chars().count();
        }
        if chunk.is_invisible && !options.include_hidden {
            continue;
        }
        let structure = marked
            .mcid
            .and_then(|mcid| structure_map.get(&(page, mcid)).cloned());
        if let Some(mcid) = marked.mcid {
            if structure.is_some() {
                mapped_mcids.insert(mcid);
            } else {
                unmapped_mcids.insert(mcid);
            }
        }
        out.push(ChunkRef {
            bbox: chunk_bbox(&chunk),
            chunk,
            original_index: idx,
            mcid: marked.mcid,
            structure,
        });
    }
    counters.mcids_mapped += mapped_mcids.len();
    counters.mcids_unmapped += unmapped_mcids.len();
    counters.struct_tree_nodes += structure_map
        .values()
        .filter(|entry| entry.page == page)
        .count();

    if options.deduplicate {
        let before = out.len();
        out = deduplicate_chunks(out);
        let removed = before.saturating_sub(out.len());
        counters.deduplicated_runs += removed;
        if removed > 0 {
            diagnostics.push(TextDiagnostic {
                code: "text.dedup.removed".to_string(),
                severity: TextDiagnosticSeverity::Info,
                page: Some(page),
                message: format!("removed {removed} duplicate text runs from semantic model"),
            });
        }
    }
    out
}

fn deduplicate_chunks(chunks: Vec<ChunkRef>) -> Vec<ChunkRef> {
    let mut kept: Vec<ChunkRef> = Vec::with_capacity(chunks.len());
    'outer: for candidate in chunks {
        for existing in &kept {
            if candidate.chunk.text == existing.chunk.text
                && (candidate.chunk.x - existing.chunk.x).abs() <= DEDUPE_X_TOLERANCE
                && (candidate.chunk.y - existing.chunk.y).abs() <= DEDUPE_Y_TOLERANCE
                && (candidate.chunk.font_size - existing.chunk.font_size).abs()
                    <= DEDUPE_FONT_TOLERANCE
                && candidate.chunk.is_invisible == existing.chunk.is_invisible
            {
                continue 'outer;
            }
        }
        kept.push(candidate);
    }
    kept
}

#[derive(Debug, Clone)]
struct BlockSpec {
    bbox: BBox,
    font_size: f64,
    lines: Vec<LineSpec>,
}

#[derive(Debug, Clone)]
struct LineSpec {
    text: String,
    bbox: BBox,
    direction: SemanticTextDirection,
}

fn layout_to_block_specs(layout: &PageLayout) -> Vec<BlockSpec> {
    layout
        .blocks
        .iter()
        .map(|block| BlockSpec {
            bbox: block.bbox,
            font_size: block.font_size,
            lines: block
                .lines
                .iter()
                .map(|line| LineSpec {
                    text: line.text.clone(),
                    bbox: line.bbox,
                    direction: if line.is_rtl {
                        SemanticTextDirection::RightToLeft
                    } else {
                        SemanticTextDirection::LeftToRight
                    },
                })
                .collect(),
        })
        .collect()
}

fn append_vertical_block_specs(blocks: &mut Vec<BlockSpec>, chunks: &[TextChunk]) {
    let vertical: Vec<TextChunk> = chunks.iter().filter(|c| c.is_vertical).cloned().collect();
    if vertical.is_empty() {
        return;
    }
    let reconstructor = ReadingOrderReconstructor::new();
    let lines = reconstructor.reconstruct(vertical);
    let mut specs = Vec::new();
    for line in lines {
        let bbox = BBox {
            x0: line.x_min,
            y0: line.y,
            x1: line.x_max,
            y1: line.y + line.font_size,
        };
        specs.push(LineSpec {
            text: line.text,
            bbox,
            direction: SemanticTextDirection::Vertical,
        });
    }
    if specs.is_empty() {
        return;
    }
    let bbox = specs
        .iter()
        .map(|line| line.bbox)
        .reduce(|acc, bbox| BBox {
            x0: acc.x0.min(bbox.x0),
            y0: acc.y0.min(bbox.y0),
            x1: acc.x1.max(bbox.x1),
            y1: acc.y1.max(bbox.y1),
        })
        .unwrap_or(BBox {
            x0: 0.0,
            y0: 0.0,
            x1: 0.0,
            y1: 0.0,
        });
    blocks.push(BlockSpec {
        bbox,
        font_size: median_font_size(chunks).unwrap_or(12.0),
        lines: specs,
    });
}

fn fallback_block_spec(chunks: &[TextChunk]) -> BlockSpec {
    let reconstructor = ReadingOrderReconstructor::new();
    let lines = reconstructor.reconstruct(chunks.to_vec());
    let mut specs = Vec::new();
    for line in lines {
        let bbox = BBox {
            x0: line.x_min,
            y0: line.y,
            x1: line.x_max,
            y1: line.y + line.font_size,
        };
        specs.push(LineSpec {
            text: line.text,
            bbox,
            direction: SemanticTextDirection::LeftToRight,
        });
    }
    let bbox = chunks
        .iter()
        .map(chunk_bbox)
        .reduce(|acc, q| TextQuad {
            x0: acc.x0.min(q.x0),
            y0: acc.y0.min(q.y0),
            x1: acc.x1.max(q.x1),
            y1: acc.y1.max(q.y1),
        })
        .map(|q| BBox {
            x0: q.x0,
            y0: q.y0,
            x1: q.x1,
            y1: q.y1,
        })
        .unwrap_or(BBox {
            x0: 0.0,
            y0: 0.0,
            x1: 0.0,
            y1: 0.0,
        });
    BlockSpec {
        bbox,
        font_size: median_font_size(chunks).unwrap_or(12.0),
        lines: specs,
    }
}

fn chunks_for_bbox(
    chunks: &[ChunkRef],
    bbox: BBox,
    direction: SemanticTextDirection,
) -> Vec<ChunkRef> {
    let mut selected: Vec<ChunkRef> = chunks
        .iter()
        .filter(|candidate| {
            candidate.bbox.intersects_bbox(bbox)
                || (matches!(direction, SemanticTextDirection::Vertical)
                    && center_y(candidate.bbox, bbox))
        })
        .cloned()
        .collect();
    match direction {
        SemanticTextDirection::RightToLeft => selected.sort_by(|a, b| {
            b.chunk
                .x
                .partial_cmp(&a.chunk.x)
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
        SemanticTextDirection::Vertical => selected.sort_by(|a, b| {
            b.chunk
                .y
                .partial_cmp(&a.chunk.y)
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
        _ => selected.sort_by(|a, b| {
            a.chunk
                .x
                .partial_cmp(&b.chunk.x)
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
    }
    selected
}

fn center_y(quad: TextQuad, bbox: BBox) -> bool {
    let cy = (quad.y0 + quad.y1) / 2.0;
    cy >= bbox.y0 - 0.5 && cy <= bbox.y1 + 0.5
}

#[allow(clippy::too_many_arguments)]
fn build_line_from_text(
    text: &str,
    bbox: BBox,
    direction: SemanticTextDirection,
    line_index: usize,
    global_char_index: &mut usize,
    global_word_index: &mut usize,
    global_span_index: &mut usize,
    options: &TextSemanticOptions,
) -> BuiltLine {
    let synthetic = TextChunk {
        text: text.to_string(),
        x: bbox.x0,
        y: bbox.y0,
        font_size: (bbox.y1 - bbox.y0).max(1.0),
        font_name: String::new(),
        width: (bbox.x1 - bbox.x0).max(0.0),
        is_rtl: matches!(direction, SemanticTextDirection::RightToLeft),
        is_vertical: matches!(direction, SemanticTextDirection::Vertical),
        is_invisible: false,
        is_actual_text: false,
        mapping_sources: Vec::new(),
    };
    build_line_from_chunks(
        &[ChunkRef {
            chunk: synthetic,
            original_index: line_index,
            bbox: TextQuad::from_bbox([bbox.x0, bbox.y0, bbox.x1, bbox.y1]),
            mcid: None,
            structure: None,
        }],
        bbox,
        direction,
        line_index,
        global_char_index,
        global_word_index,
        global_span_index,
        options,
    )
}

#[allow(clippy::too_many_arguments)]
fn build_line_from_chunks(
    chunks: &[ChunkRef],
    bbox: BBox,
    direction: SemanticTextDirection,
    _line_index: usize,
    global_char_index: &mut usize,
    global_word_index: &mut usize,
    global_span_index: &mut usize,
    options: &TextSemanticOptions,
) -> BuiltLine {
    let mut chars = Vec::new();
    let mut spans = Vec::new();
    let mut words = Vec::new();
    let mut text_parts: Vec<String> = Vec::new();

    for chunk_ref in chunks {
        let chunk = &chunk_ref.chunk;
        if chunk.text.trim().is_empty() {
            continue;
        }
        if !text_parts.is_empty() && needs_space(text_parts.last().unwrap(), &chunk.text, direction)
        {
            text_parts.push(" ".to_string());
            let quad = chars
                .last()
                .map(|last: &TextSemanticChar| TextQuad {
                    x0: last.quad.x1,
                    y0: last.quad.y0,
                    x1: chunk_ref.bbox.x0.max(last.quad.x1),
                    y1: last.quad.y1,
                })
                .unwrap_or(chunk_ref.bbox);
            chars.push(TextSemanticChar {
                text: " ".to_string(),
                unicode: " ".to_string(),
                char_index: *global_char_index,
                chunk_index: chunk_ref.original_index,
                font_name: chunk.font_name.clone(),
                font_size: chunk.font_size,
                direction,
                mapping_source: TextMappingSource::NativePdfText,
                provenance: vec![TextProvenanceFlag::SyntheticLayout],
                mcid: None,
                struct_role: None,
                original_role: None,
                role_source: TextRoleSource::Synthetic,
                quad,
                confidence: 0.62,
            });
            *global_char_index += 1;
        }
        text_parts.push(chunk.text.clone());

        let start_char = *global_char_index;
        let chunk_chars = char_quads_for_chunk(chunk, chunk_ref.original_index, global_char_index);
        let mut span_chars = Vec::new();
        for (char_offset, ch, quad, char_index) in chunk_chars {
            let mapping_source = mapping_source_for_char(chunk, char_offset);
            let mut provenance = provenance_for_char(chunk, char_offset);
            if chunk_ref.mcid.is_some() {
                provenance.push(TextProvenanceFlag::TaggedMcid);
            }
            if chunk_ref.structure.is_some() {
                provenance.push(TextProvenanceFlag::TaggedPdf);
                provenance.push(TextProvenanceFlag::StructTreeRole);
            }
            provenance = flags_union(&provenance);
            let (struct_role, original_role, role_source) =
                structure_role_fields(&chunk_ref.structure);
            span_chars.push(TextSemanticChar {
                text: ch.to_string(),
                unicode: ch.to_string(),
                char_index,
                chunk_index: chunk_ref.original_index,
                font_name: chunk.font_name.clone(),
                font_size: chunk.font_size,
                direction: direction_for_chunk(chunk, direction),
                mapping_source,
                provenance,
                mcid: chunk_ref.mcid,
                struct_role,
                original_role,
                role_source,
                quad,
                confidence: if ch == '\u{FFFD}' { 0.1 } else { 0.82 },
            });
        }
        let end_char = *global_char_index;
        let span_quad = TextQuad::union(&span_chars.iter().map(|ch| ch.quad).collect::<Vec<_>>())
            .unwrap_or(chunk_ref.bbox);
        spans.push(TextSemanticSpan {
            text: chunk.text.clone(),
            span_index: *global_span_index,
            char_range: [start_char, end_char],
            quad: span_quad,
            font_name: chunk.font_name.clone(),
            font_size: chunk.font_size,
            direction: direction_for_chunk(chunk, direction),
            mapping_source: aggregate_mapping_source(&span_chars),
            provenance: flags_union(
                &span_chars
                    .iter()
                    .flat_map(|ch| ch.provenance.iter().copied())
                    .collect::<Vec<_>>(),
            ),
            provenance_summary: provenance_summary_for_chars(&span_chars),
            mcids: mcids_for_chars(&span_chars),
            struct_role: chunk_ref
                .structure
                .as_ref()
                .map(|entry| entry.normalized_role.clone()),
            original_role: chunk_ref
                .structure
                .as_ref()
                .map(|entry| entry.original_role.clone()),
            role_source: chunk_ref
                .structure
                .as_ref()
                .map(|entry| entry.role_source)
                .unwrap_or(TextRoleSource::Unknown),
            confidence: if chunk.text.contains('\u{FFFD}') {
                0.2
            } else {
                0.82
            },
        });
        *global_span_index += 1;
        chars.extend(span_chars);
    }

    let line_text = text_parts.join("").trim().to_string();
    let token_ranges = tokenize_words_from_chars(chars.as_slice(), options);
    for (word_text, start, end) in token_ranges {
        let word_semantic_chars: Vec<&TextSemanticChar> = chars
            .iter()
            .filter(|ch| ch.char_index >= start && ch.char_index < end)
            .collect();
        let word_chars: Vec<TextQuad> = chars
            .iter()
            .filter(|ch| ch.char_index >= start && ch.char_index < end)
            .map(|ch| ch.quad)
            .collect();
        let quad = TextQuad::union(&word_chars)
            .unwrap_or_else(|| TextQuad::from_bbox([bbox.x0, bbox.y0, bbox.x1, bbox.y1]));
        let mut provenance = flags_union(
            &word_semantic_chars
                .iter()
                .flat_map(|ch| ch.provenance.iter().copied())
                .collect::<Vec<_>>(),
        );
        if matches!(options.cjk_segmentation, CjkSegmentationMode::Dictionary)
            && word_text.chars().any(is_cjk_char)
            && !provenance.contains(&TextProvenanceFlag::DictionarySegmented)
        {
            provenance.push(TextProvenanceFlag::DictionarySegmented);
        }
        words.push(TextSemanticWord {
            text: word_text,
            word_index: *global_word_index,
            char_range: [start, end],
            quad,
            confidence: 0.84,
            provenance,
            provenance_summary: provenance_summary_for_char_refs(&word_semantic_chars),
            mcids: mcids_for_char_refs(&word_semantic_chars),
        });
        *global_word_index += 1;
    }

    let line_quad = TextQuad::union(&chars.iter().map(|ch| ch.quad).collect::<Vec<_>>())
        .unwrap_or_else(|| TextQuad::from_bbox([bbox.x0, bbox.y0, bbox.x1, bbox.y1]));
    let mut provenance = flags_union(
        &spans
            .iter()
            .flat_map(|span| span.provenance.iter().copied())
            .collect::<Vec<_>>(),
    );
    provenance.push(TextProvenanceFlag::SyntheticLayout);
    provenance = flags_union(&provenance);

    BuiltLine {
        text: if line_text.is_empty() {
            chunks
                .iter()
                .map(|c| c.chunk.text.as_str())
                .collect::<Vec<_>>()
                .join("")
        } else {
            line_text
        },
        direction,
        words,
        spans,
        chars: if options.include_chars {
            chars
        } else {
            Vec::new()
        },
        quad: line_quad,
        provenance,
    }
}

fn char_quads_for_chunk(
    chunk: &TextChunk,
    chunk_index: usize,
    global_char_index: &mut usize,
) -> Vec<(usize, char, TextQuad, usize)> {
    let chars: Vec<char> = chunk.text.chars().collect();
    if chars.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(chars.len());
    let count = chars.len().max(1) as f64;
    if chunk.is_vertical {
        let step = (chunk.width.max(chunk.font_size) / count).max(0.1);
        for (idx, ch) in chars.into_iter().enumerate() {
            let y1 = chunk.y - step * idx as f64;
            let y0 = y1 - step;
            let quad = TextQuad {
                x0: chunk.x,
                y0: y0.min(y1),
                x1: chunk.x + chunk.font_size.max(1.0),
                y1: y0.max(y1) + chunk.font_size.min(step),
            };
            out.push((idx, ch, quad, *global_char_index));
            *global_char_index += 1;
        }
    } else {
        let width = chunk.width.max(chunk.font_size * count * 0.35);
        let step = width / count;
        for (idx, ch) in chars.into_iter().enumerate() {
            let x0 = chunk.x + step * idx as f64;
            let x1 = if idx + 1 == count as usize {
                chunk.x + width
            } else {
                x0 + step
            };
            let quad = TextQuad {
                x0,
                y0: chunk.y,
                x1,
                y1: chunk.y + chunk.font_size.max(1.0),
            };
            out.push((idx, ch, quad, *global_char_index));
            *global_char_index += 1;
        }
    }
    let _ = chunk_index;
    out
}

fn tokenize_words_from_chars(
    chars: &[TextSemanticChar],
    options: &TextSemanticOptions,
) -> Vec<(String, usize, usize)> {
    if matches!(options.cjk_segmentation, CjkSegmentationMode::Dictionary) {
        return tokenize_words_from_chars_dictionary(chars, options);
    }

    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut start = None;
    let mut cjk_run_script: Option<CjkScript> = None;

    for ch in chars {
        let Some(c) = ch.text.chars().next() else {
            continue;
        };
        if c.is_whitespace() {
            flush_token(&mut tokens, &mut current, &mut start, ch.char_index);
            cjk_run_script = None;
            continue;
        }
        if is_cjk_char(c) {
            match options.cjk_segmentation {
                CjkSegmentationMode::Char => {
                    flush_token(&mut tokens, &mut current, &mut start, ch.char_index);
                    tokens.push((c.to_string(), ch.char_index, ch.char_index + 1));
                }
                CjkSegmentationMode::Simple | CjkSegmentationMode::Dictionary => {
                    let script = cjk_script(c);
                    let over_cap = current.chars().count() >= options.max_cjk_run_chars.max(1);
                    if cjk_run_script.is_some_and(|active| active != script) || over_cap {
                        flush_token(&mut tokens, &mut current, &mut start, ch.char_index);
                    }
                    cjk_run_script = Some(script);
                    if start.is_none() {
                        start = Some(ch.char_index);
                    }
                    current.push(c);
                }
            }
            continue;
        }
        if cjk_run_script.is_some() {
            flush_token(&mut tokens, &mut current, &mut start, ch.char_index);
        }
        cjk_run_script = None;
        if is_cjk_punctuation(c) {
            flush_token(&mut tokens, &mut current, &mut start, ch.char_index);
            continue;
        }
        if start.is_none() {
            start = Some(ch.char_index);
        }
        current.push(c);
    }
    let end = chars.last().map(|ch| ch.char_index + 1).unwrap_or(0);
    flush_token(&mut tokens, &mut current, &mut start, end);
    tokens
}

fn tokenize_words_from_chars_dictionary(
    chars: &[TextSemanticChar],
    options: &TextSemanticOptions,
) -> Vec<(String, usize, usize)> {
    let mut tokens = Vec::new();
    let mut latin = String::new();
    let mut latin_start = None;
    let mut cjk_run: Vec<&TextSemanticChar> = Vec::new();
    let mut cjk_run_script: Option<CjkScript> = None;
    let provider = CjkDictionaryProvider::builtin_fixture();

    for ch in chars {
        let Some(c) = ch.text.chars().next() else {
            continue;
        };
        if c.is_whitespace() {
            flush_token(&mut tokens, &mut latin, &mut latin_start, ch.char_index);
            flush_dictionary_cjk_run(&mut tokens, &mut cjk_run, &provider);
            cjk_run_script = None;
            continue;
        }
        if is_cjk_char(c) {
            flush_token(&mut tokens, &mut latin, &mut latin_start, ch.char_index);
            let script = cjk_script(c);
            let over_cap = cjk_run.len() >= options.max_cjk_run_chars.max(1);
            if cjk_run_script.is_some_and(|active| !dictionary_scripts_compatible(active, script))
                || over_cap
            {
                flush_dictionary_cjk_run(&mut tokens, &mut cjk_run, &provider);
            }
            cjk_run_script = Some(script);
            cjk_run.push(ch);
            continue;
        }
        flush_dictionary_cjk_run(&mut tokens, &mut cjk_run, &provider);
        cjk_run_script = None;
        if is_cjk_punctuation(c) {
            flush_token(&mut tokens, &mut latin, &mut latin_start, ch.char_index);
            tokens.push((c.to_string(), ch.char_index, ch.char_index + 1));
            continue;
        }
        if latin_start.is_none() {
            latin_start = Some(ch.char_index);
        }
        latin.push(c);
    }
    let end = chars.last().map(|ch| ch.char_index + 1).unwrap_or(0);
    flush_token(&mut tokens, &mut latin, &mut latin_start, end);
    flush_dictionary_cjk_run(&mut tokens, &mut cjk_run, &provider);
    tokens
}

fn flush_dictionary_cjk_run(
    tokens: &mut Vec<(String, usize, usize)>,
    run: &mut Vec<&TextSemanticChar>,
    provider: &CjkDictionaryProvider,
) {
    if run.is_empty() {
        return;
    }
    let chars: Vec<char> = run.iter().filter_map(|ch| ch.text.chars().next()).collect();
    let mut index = 0;
    while index < chars.len() {
        let len = provider
            .best_match(&chars, index)
            .map(|entry| entry.chars.len())
            .unwrap_or(1);
        let start = run[index].char_index;
        let end = run[index + len - 1].char_index + 1;
        let text: String = chars[index..index + len].iter().collect();
        tokens.push((text, start, end));
        index += len;
    }
    run.clear();
}

fn resolve_manifest_entries_path(manifest_path: &std::path::Path, entries_path: &str) -> PathBuf {
    let candidate = PathBuf::from(entries_path);
    if candidate.is_absolute() {
        candidate
    } else {
        manifest_path
            .parent()
            .map(|parent| parent.join(candidate.clone()))
            .unwrap_or(candidate)
    }
}

fn parse_dictionary_tsv_entries(
    manifest: &CjkDictionaryPackManifest,
    entries_bytes: &[u8],
    max_token_chars: usize,
    malformed: &mut usize,
) -> Result<Vec<IndexedCjkDictionaryEntry>> {
    let text = std::str::from_utf8(entries_bytes).map_err(|err| {
        OxideError::invalid_input(format!(
            "CJK dictionary pack {} entries are not UTF-8: {err}",
            manifest.pack_id
        ))
    })?;
    let mut entries = Vec::new();
    for (line_index, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let columns = line.split('\t').map(str::trim).collect::<Vec<_>>();
        if columns.len() < 2 {
            *malformed += 1;
            continue;
        }
        let term = normalize_dictionary_term(columns[0]);
        let language = columns[1].to_ascii_lowercase();
        if term.is_empty()
            || term.chars().count() > max_token_chars
            || !is_supported_dictionary_language(&language)
        {
            *malformed += 1;
            continue;
        }
        let priority = columns
            .get(2)
            .and_then(|value| value.parse::<i32>().ok())
            .unwrap_or(0);
        let source = columns
            .get(3)
            .filter(|value| !value.is_empty())
            .map(|value| (*value).to_string())
            .unwrap_or_else(|| manifest.pack_id.clone());
        let confidence = columns
            .get(4)
            .and_then(|value| value.parse::<f32>().ok())
            .unwrap_or(0.91)
            .clamp(0.0, 1.0);
        entries.push(IndexedCjkDictionaryEntry {
            chars: term.chars().collect(),
            term,
            language,
            priority,
            source,
            confidence,
            ordinal: line_index,
        });
    }
    let parsed_count = entries.len();
    if parsed_count != manifest.entry_count {
        return Err(OxideError::invalid_input(format!(
            "CJK dictionary pack {} manifest entry_count {} does not match parsed valid entry count {}",
            manifest.pack_id, manifest.entry_count, parsed_count
        )));
    }
    Ok(entries)
}

fn dedupe_and_order_dictionary_entries(
    mut entries: Vec<IndexedCjkDictionaryEntry>,
) -> (Vec<IndexedCjkDictionaryEntry>, usize) {
    entries.sort_by(|left, right| {
        left.language
            .cmp(&right.language)
            .then_with(|| left.term.cmp(&right.term))
            .then_with(|| right.priority.cmp(&left.priority))
            .then_with(|| left.source.cmp(&right.source))
            .then_with(|| left.ordinal.cmp(&right.ordinal))
    });
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut duplicate_count = 0usize;
    for entry in entries {
        let key = (entry.term.clone(), entry.language.clone());
        if seen.insert(key) {
            out.push(entry);
        } else {
            duplicate_count += 1;
        }
    }
    for (ordinal, entry) in out.iter_mut().enumerate() {
        entry.ordinal = ordinal;
    }
    (out, duplicate_count)
}

fn normalize_dictionary_term(term: &str) -> String {
    term.trim().to_string()
}

fn is_supported_dictionary_language(language: &str) -> bool {
    matches!(
        language,
        "zh" | "ja" | "ko" | "mixed" | "mixed_latin" | "und"
    )
}

pub fn cjk_dictionary_entries_sha256(entries_bytes: &[u8]) -> String {
    sha256_digest(entries_bytes)
}

fn sha256_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(71);
    out.push_str("sha256:");
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CjkScript {
    Han,
    Hiragana,
    Katakana,
    Hangul,
    Other,
}

fn cjk_script(c: char) -> CjkScript {
    let cp = c as u32;
    match cp {
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF => CjkScript::Han,
        0x3040..=0x309F => CjkScript::Hiragana,
        0x30A0..=0x30FF => CjkScript::Katakana,
        0xAC00..=0xD7AF => CjkScript::Hangul,
        _ => CjkScript::Other,
    }
}

fn language_for_cjk_char(c: char) -> &'static str {
    match cjk_script(c) {
        CjkScript::Han => "zh",
        CjkScript::Hiragana | CjkScript::Katakana => "ja",
        CjkScript::Hangul => "ko",
        CjkScript::Other => "und",
    }
}

fn dictionary_scripts_compatible(left: CjkScript, right: CjkScript) -> bool {
    left == right
        || matches!(
            (left, right),
            (CjkScript::Han, CjkScript::Hiragana)
                | (CjkScript::Hiragana, CjkScript::Han)
                | (CjkScript::Han, CjkScript::Katakana)
                | (CjkScript::Katakana, CjkScript::Han)
                | (CjkScript::Hiragana, CjkScript::Katakana)
                | (CjkScript::Katakana, CjkScript::Hiragana)
        )
}

fn builtin_cjk_dictionary_hash() -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for entry in BUILTIN_CJK_DICTIONARY {
        for b in entry
            .term
            .as_bytes()
            .iter()
            .chain(entry.language.as_bytes())
        {
            hash ^= u64::from(*b);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    format!("fnv1a64:{hash:016x}")
}

fn is_cjk_punctuation(c: char) -> bool {
    matches!(
        c,
        '\u{3000}'..='\u{303F}' | '\u{FF00}'..='\u{FFEF}'
    )
}

fn flush_token(
    tokens: &mut Vec<(String, usize, usize)>,
    current: &mut String,
    start: &mut Option<usize>,
    end: usize,
) {
    if !current.is_empty() {
        tokens.push((current.clone(), start.unwrap_or(end), end));
        current.clear();
    }
    *start = None;
}

fn is_cjk_char(c: char) -> bool {
    let cp = c as u32;
    matches!(
        cp,
        0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF
            | 0x3040..=0x30FF
            | 0xAC00..=0xD7AF
    )
}

fn needs_space(left: &str, right: &str, direction: SemanticTextDirection) -> bool {
    if matches!(direction, SemanticTextDirection::Vertical) {
        return false;
    }
    let Some(last) = left.chars().rev().find(|c| !c.is_whitespace()) else {
        return false;
    };
    let Some(first) = right.chars().find(|c| !c.is_whitespace()) else {
        return false;
    };
    !last.is_whitespace()
        && !first.is_whitespace()
        && !is_cjk_char(last)
        && !is_cjk_char(first)
        && last != '-'
}

fn build_paragraphs(lines: &[TextSemanticLine], role: TextRole) -> Vec<TextSemanticParagraph> {
    if lines.is_empty() {
        return Vec::new();
    }
    let mut paragraphs = Vec::new();
    let mut start = 0usize;
    for idx in 1..lines.len() {
        let prev = lines[idx - 1].quad;
        let current = lines[idx].quad;
        let prev_height = (prev.y1 - prev.y0).max(1.0);
        let gap = prev.y0 - current.y1;
        let indent_delta = (current.x0 - lines[start].quad.x0).abs();
        if gap > prev_height * 0.9 || indent_delta > prev_height * 2.0 {
            push_paragraph(&mut paragraphs, lines, start, idx, role);
            start = idx;
        }
    }
    push_paragraph(&mut paragraphs, lines, start, lines.len(), role);
    paragraphs
}

fn push_paragraph(
    out: &mut Vec<TextSemanticParagraph>,
    lines: &[TextSemanticLine],
    start: usize,
    end: usize,
    role: TextRole,
) {
    let line_slice = &lines[start..end];
    let text = line_slice
        .iter()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let quad = TextQuad::union(&line_slice.iter().map(|line| line.quad).collect::<Vec<_>>())
        .unwrap_or(TextQuad {
            x0: 0.0,
            y0: 0.0,
            x1: 0.0,
            y1: 0.0,
        });
    out.push(TextSemanticParagraph {
        text,
        paragraph_index: out.len(),
        line_range: [start, end],
        role,
        quad,
        confidence: 0.72,
        role_source: TextRoleSource::Heuristic,
        role_confidence: 0.58,
    });
}

fn classify_block(
    bbox: BBox,
    font_size: f64,
    median_font_size: f64,
    page_box: [f64; 4],
    page_height: f64,
) -> TextRole {
    let top_band = page_box[3] - page_height * 0.08;
    let bottom_band = page_box[1] + page_height * 0.08;
    let furniture_like_height = bbox.height() <= median_font_size.max(1.0) * 2.5;
    if bbox.y1 >= top_band && furniture_like_height {
        return TextRole::Header;
    }
    if bbox.y0 <= bottom_band && furniture_like_height {
        return TextRole::Footer;
    }
    if font_size > median_font_size * 1.25 {
        return TextRole::Heading;
    }
    if font_size < median_font_size * 0.82 {
        return TextRole::Footnote;
    }
    TextRole::BodyText
}

fn classify_line(
    text: &str,
    block_role: TextRole,
    quad: TextQuad,
    page_box: [f64; 4],
    median_font_size: f64,
) -> TextRole {
    let trimmed = text.trim_start();
    if trimmed.starts_with(['-', '*', '\u{2022}']) || starts_with_numbered_list(trimmed) {
        return TextRole::List;
    }
    if trimmed.to_ascii_lowercase().starts_with("figure ")
        || trimmed.to_ascii_lowercase().starts_with("fig. ")
        || trimmed.to_ascii_lowercase().starts_with("table ")
    {
        return TextRole::FigureCaption;
    }
    if (quad.y1 - quad.y0) < median_font_size * 0.85
        && quad.y0 < page_box[1] + (page_box[3] - page_box[1]).abs() * 0.25
    {
        return TextRole::Footnote;
    }
    block_role
}

fn starts_with_numbered_list(text: &str) -> bool {
    let mut chars = text.chars().peekable();
    let mut saw_digit = false;
    while matches!(chars.peek(), Some(c) if c.is_ascii_digit()) {
        saw_digit = true;
        chars.next();
    }
    saw_digit && matches!(chars.next(), Some('.') | Some(')'))
}

fn mapping_source_for_char(chunk: &TextChunk, char_offset: usize) -> TextMappingSource {
    if chunk.is_actual_text {
        return TextMappingSource::ActualText;
    }
    if chunk.is_invisible {
        return TextMappingSource::Ocr;
    }
    match chunk.mapping_sources.get(char_offset).copied() {
        Some(FontDecodeSource::ActualText) => TextMappingSource::ActualText,
        Some(FontDecodeSource::ToUnicode) => TextMappingSource::ToUnicode,
        Some(FontDecodeSource::PredefinedCMap) => TextMappingSource::PredefinedCMap,
        Some(FontDecodeSource::EncodingDifferences) => TextMappingSource::EncodingDifferences,
        Some(FontDecodeSource::GlyphName) => TextMappingSource::GlyphName,
        Some(FontDecodeSource::FontCMap) => TextMappingSource::FontCMap,
        Some(FontDecodeSource::IdentityCid) => TextMappingSource::IdentityCid,
        Some(FontDecodeSource::NativePdfText) => TextMappingSource::NativePdfText,
        Some(FontDecodeSource::Unknown) => TextMappingSource::Unknown,
        None if chunk.text.contains('\u{FFFD}') => TextMappingSource::Unknown,
        None => TextMappingSource::NativePdfText,
    }
}

fn provenance_for_char(chunk: &TextChunk, char_offset: usize) -> Vec<TextProvenanceFlag> {
    let mut flags = Vec::new();
    flags.push(match mapping_source_for_char(chunk, char_offset) {
        TextMappingSource::ActualText => TextProvenanceFlag::ActualText,
        TextMappingSource::ToUnicode => TextProvenanceFlag::ToUnicode,
        TextMappingSource::PredefinedCMap => TextProvenanceFlag::PredefinedCMap,
        TextMappingSource::EmbeddedCMap => TextProvenanceFlag::FallbackCMap,
        TextMappingSource::EncodingDifferences => TextProvenanceFlag::EncodingDifferences,
        TextMappingSource::GlyphName | TextMappingSource::UniName => {
            TextProvenanceFlag::FallbackGlyphName
        }
        TextMappingSource::FontCMap => TextProvenanceFlag::FontCMap,
        TextMappingSource::IdentityCid => TextProvenanceFlag::IdentityCid,
        TextMappingSource::Ocr => TextProvenanceFlag::Ocr,
        TextMappingSource::Unknown => TextProvenanceFlag::UnknownUnmapped,
        TextMappingSource::TaggedPdf | TextMappingSource::NativePdfText => {
            TextProvenanceFlag::NativePdfText
        }
    });
    if chunk.is_invisible {
        flags.push(TextProvenanceFlag::HiddenOrInvisible);
        flags.push(TextProvenanceFlag::Ocr);
    }
    if chunk
        .text
        .chars()
        .nth(char_offset)
        .is_some_and(|ch| matches!(ch, '\u{FB00}'..='\u{FB06}'))
    {
        flags.push(TextProvenanceFlag::LigatureExpansion);
    }
    flags
}

fn structure_role_fields(
    entry: &Option<TextStructureEntry>,
) -> (Option<String>, Option<String>, TextRoleSource) {
    entry
        .as_ref()
        .map(|entry| {
            (
                Some(entry.normalized_role.clone()),
                Some(entry.original_role.clone()),
                entry.role_source,
            )
        })
        .unwrap_or((None, None, TextRoleSource::Unknown))
}

fn aggregate_mapping_source(chars: &[TextSemanticChar]) -> TextMappingSource {
    let mut counts: HashMap<TextMappingSource, usize> = HashMap::new();
    for ch in chars {
        *counts.entry(ch.mapping_source).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(source, _)| source)
        .unwrap_or(TextMappingSource::Unknown)
}

fn provenance_summary_for_chars(chars: &[TextSemanticChar]) -> TextProvenanceSummary {
    let refs: Vec<&TextSemanticChar> = chars.iter().collect();
    provenance_summary_for_char_refs(&refs)
}

fn provenance_summary_for_char_refs(chars: &[&TextSemanticChar]) -> TextProvenanceSummary {
    let mut summary = TextProvenanceSummary::default();
    for ch in chars {
        add_char_to_summary(&mut summary, ch);
    }
    summary
}

fn provenance_summary_for_spans(spans: &[TextSemanticSpan]) -> TextProvenanceSummary {
    let mut summary = TextProvenanceSummary::default();
    for span in spans {
        merge_summary(&mut summary, &span.provenance_summary);
    }
    summary
}

fn provenance_summary_for_lines(lines: &[TextSemanticLine]) -> TextProvenanceSummary {
    let mut summary = TextProvenanceSummary::default();
    for line in lines {
        merge_summary(&mut summary, &line.provenance_summary);
    }
    summary
}

fn add_char_to_summary(summary: &mut TextProvenanceSummary, ch: &TextSemanticChar) {
    match ch.mapping_source {
        TextMappingSource::ActualText => summary.actual_text += 1,
        TextMappingSource::ToUnicode => summary.tounicode += 1,
        TextMappingSource::EmbeddedCMap => summary.embedded_cmap += 1,
        TextMappingSource::PredefinedCMap => summary.predefined_cmap += 1,
        TextMappingSource::EncodingDifferences => summary.encoding_differences += 1,
        TextMappingSource::GlyphName | TextMappingSource::UniName => summary.glyph_name += 1,
        TextMappingSource::FontCMap => summary.font_cmap += 1,
        TextMappingSource::IdentityCid => summary.identity_cid += 1,
        TextMappingSource::Ocr => summary.ocr += 1,
        TextMappingSource::Unknown => summary.unknown_unmapped += 1,
        TextMappingSource::NativePdfText | TextMappingSource::TaggedPdf => {}
    }
    for flag in &ch.provenance {
        match flag {
            TextProvenanceFlag::SyntheticLayout => summary.synthetic_layout += 1,
            TextProvenanceFlag::HiddenOrInvisible => summary.hidden_or_invisible += 1,
            TextProvenanceFlag::TaggedMcid => summary.tagged_mcid += 1,
            TextProvenanceFlag::HeuristicRole => summary.heuristic_role += 1,
            TextProvenanceFlag::LigatureExpansion => summary.ligature_expansion += 1,
            TextProvenanceFlag::HyphenationJoin => summary.hyphenation_join += 1,
            TextProvenanceFlag::NormalizedWhitespace => summary.normalized_whitespace += 1,
            _ => {}
        }
    }
}

fn merge_summary(into: &mut TextProvenanceSummary, other: &TextProvenanceSummary) {
    into.actual_text += other.actual_text;
    into.tounicode += other.tounicode;
    into.embedded_cmap += other.embedded_cmap;
    into.predefined_cmap += other.predefined_cmap;
    into.encoding_differences += other.encoding_differences;
    into.glyph_name += other.glyph_name;
    into.font_cmap += other.font_cmap;
    into.identity_cid += other.identity_cid;
    into.ocr += other.ocr;
    into.synthetic_layout += other.synthetic_layout;
    into.hidden_or_invisible += other.hidden_or_invisible;
    into.unknown_unmapped += other.unknown_unmapped;
    into.tagged_mcid += other.tagged_mcid;
    into.heuristic_role += other.heuristic_role;
    into.ligature_expansion += other.ligature_expansion;
    into.hyphenation_join += other.hyphenation_join;
    into.normalized_whitespace += other.normalized_whitespace;
}

fn mcids_for_chars(chars: &[TextSemanticChar]) -> Vec<i64> {
    let refs: Vec<&TextSemanticChar> = chars.iter().collect();
    mcids_for_char_refs(&refs)
}

fn mcids_for_char_refs(chars: &[&TextSemanticChar]) -> Vec<i64> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for ch in chars {
        if let Some(mcid) = ch.mcid {
            if seen.insert(mcid) {
                out.push(mcid);
            }
        }
    }
    out
}

fn mcids_for_spans(spans: &[TextSemanticSpan]) -> Vec<i64> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for span in spans {
        for mcid in &span.mcids {
            if seen.insert(*mcid) {
                out.push(*mcid);
            }
        }
    }
    out
}

fn mcids_for_lines(lines: &[TextSemanticLine]) -> Vec<i64> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for line in lines {
        for mcid in &line.mcids {
            if seen.insert(*mcid) {
                out.push(*mcid);
            }
        }
    }
    out
}

fn role_from_spans(spans: &[TextSemanticSpan]) -> Option<(TextRole, TextRoleSource, f32)> {
    let mut counts: HashMap<TextRole, (usize, TextRoleSource)> = HashMap::new();
    for span in spans {
        let Some(role) = span
            .struct_role
            .as_ref()
            .map(|role| text_role_from_tag(role))
            .filter(|role| *role != TextRole::Unknown)
        else {
            continue;
        };
        let entry = counts.entry(role).or_insert((0, span.role_source));
        entry.0 += 1;
    }
    counts
        .into_iter()
        .max_by_key(|(_, (count, _))| *count)
        .map(|(role, (_, source))| {
            (
                role,
                source,
                if source == TextRoleSource::Tagged {
                    0.94
                } else {
                    0.82
                },
            )
        })
}

fn role_from_lines(lines: &[TextSemanticLine]) -> Option<(TextRole, TextRoleSource, f32)> {
    let mut counts: HashMap<TextRole, (usize, TextRoleSource, f32)> = HashMap::new();
    for line in lines {
        if !matches!(
            line.role_source,
            TextRoleSource::Tagged | TextRoleSource::RoleMap
        ) {
            continue;
        }
        let entry = counts
            .entry(line.role)
            .or_insert((0, line.role_source, line.role_confidence));
        entry.0 += 1;
        entry.2 = entry.2.max(line.role_confidence);
    }
    counts
        .into_iter()
        .max_by_key(|(_, (count, _, _))| *count)
        .map(|(role, (_, source, confidence))| (role, source, confidence))
}

fn block_structure_roles(lines: &[TextSemanticLine]) -> (Option<String>, Option<String>) {
    for line in lines {
        for span in &line.spans {
            if span.struct_role.is_some() || span.original_role.is_some() {
                return (span.struct_role.clone(), span.original_role.clone());
            }
        }
    }
    (None, None)
}

pub fn text_role_from_tag(role: &str) -> TextRole {
    match role {
        "H" | "H1" | "H2" | "H3" | "H4" | "H5" | "H6" => TextRole::Heading,
        "L" | "LI" | "Lbl" | "LBody" => TextRole::List,
        "Table" | "TR" | "TH" | "TD" | "THead" | "TBody" | "TFoot" => TextRole::TableCandidate,
        "Figure" | "Caption" => TextRole::FigureCaption,
        "Note" | "Reference" => TextRole::Footnote,
        "Artifact" => TextRole::Unknown,
        "P" | "Span" | "Sect" | "Part" | "Document" | "Div" => TextRole::BodyText,
        _ => TextRole::Unknown,
    }
}

fn direction_for_chunk(
    chunk: &TextChunk,
    fallback: SemanticTextDirection,
) -> SemanticTextDirection {
    if chunk.is_vertical {
        SemanticTextDirection::Vertical
    } else if chunk.is_rtl {
        SemanticTextDirection::RightToLeft
    } else {
        fallback
    }
}

fn chunk_bbox(chunk: &TextChunk) -> TextQuad {
    if chunk.is_vertical {
        TextQuad {
            x0: chunk.x,
            y0: chunk.y - chunk.width.max(0.0),
            x1: chunk.x + chunk.font_size.max(1.0),
            y1: chunk.y + chunk.font_size.max(1.0),
        }
    } else {
        TextQuad {
            x0: chunk.x,
            y0: chunk.y,
            x1: chunk.x + chunk.width.max(0.0),
            y1: chunk.y + chunk.font_size.max(1.0),
        }
    }
}

fn median_font_size(chunks: &[TextChunk]) -> Option<f64> {
    let mut sizes: Vec<f64> = chunks
        .iter()
        .map(|chunk| chunk.font_size)
        .filter(|value| value.is_finite() && *value > 0.0)
        .collect();
    if sizes.is_empty() {
        return None;
    }
    sizes.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(sizes[sizes.len() / 2])
}

fn flags_union(flags: &[TextProvenanceFlag]) -> Vec<TextProvenanceFlag> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for flag in flags {
        if seen.insert(*flag) {
            out.push(*flag);
        }
    }
    out
}

fn merge_counters(into: &mut TextExtractionCounters, other: &TextExtractionCounters) {
    into.pages += other.pages;
    into.blocks += other.blocks;
    into.lines += other.lines;
    into.words += other.words;
    into.chars += other.chars;
    into.total_glyph_runs += other.total_glyph_runs;
    into.mapped_via_tounicode += other.mapped_via_tounicode;
    into.mapped_via_actual_text += other.mapped_via_actual_text;
    into.mapped_via_cmap += other.mapped_via_cmap;
    into.mapped_via_encoding_differences += other.mapped_via_encoding_differences;
    into.mapped_via_glyph_name += other.mapped_via_glyph_name;
    into.mapped_via_ocr += other.mapped_via_ocr;
    into.unknown_unmapped += other.unknown_unmapped;
    into.hidden_or_invisible += other.hidden_or_invisible;
    into.rtl_runs += other.rtl_runs;
    into.vertical_runs += other.vertical_runs;
    into.deduplicated_runs += other.deduplicated_runs;
    into.low_confidence_order_edges += other.low_confidence_order_edges;
    into.struct_tree_nodes += other.struct_tree_nodes;
    into.mcids_mapped += other.mcids_mapped;
    into.mcids_unmapped += other.mcids_unmapped;
    into.cjk_tokens += other.cjk_tokens;
    into.cjk_simple_tokens += other.cjk_simple_tokens;
    into.cjk_dictionary_tokens += other.cjk_dictionary_tokens;
}

fn search_semantic_document(
    document: &TextSemanticDocument,
    query: &str,
    options: &TextSearchOptions,
) -> Vec<TextSearchMatch> {
    let query_norm = normalize_query(query, options);
    if query_norm.is_empty() {
        return Vec::new();
    }
    let query_chars = query_norm.chars().collect::<Vec<_>>();

    let mut matches = Vec::new();
    for page in &document.pages {
        let stream = searchable_stream(page, options);
        let mut start = 0usize;
        while matches.len() < options.max_matches {
            let Some(pos) = stream[start..]
                .windows(query_chars.len())
                .position(|window| {
                    window
                        .iter()
                        .map(|item| item.ch)
                        .eq(query_chars.iter().copied())
                })
            else {
                break;
            };
            let from = start + pos;
            let to = from + query_chars.len();
            let char_refs: Vec<&TextSemanticChar> = stream[from..to]
                .iter()
                .filter_map(|item| item.char_ref)
                .collect();
            if !char_refs.is_empty() {
                let mut seen = HashSet::new();
                let unique_refs: Vec<&TextSemanticChar> = char_refs
                    .into_iter()
                    .filter(|ch| seen.insert(ch.char_index))
                    .collect();
                let quads = unique_refs.iter().map(|ch| ch.quad).collect::<Vec<_>>();
                let text = unique_refs
                    .iter()
                    .map(|ch| ch.text.as_str())
                    .collect::<Vec<_>>()
                    .join("");
                let provenance = flags_union(
                    &unique_refs
                        .iter()
                        .flat_map(|ch| ch.provenance.iter().copied())
                        .collect::<Vec<_>>(),
                );
                let first = unique_refs.first().map(|ch| ch.char_index).unwrap_or(0);
                let last = unique_refs
                    .last()
                    .map(|ch| ch.char_index + 1)
                    .unwrap_or(first);
                matches.push(TextSearchMatch {
                    page: page.page,
                    text,
                    normalized_text: query_norm.clone(),
                    char_range: [first, last],
                    quads,
                    confidence: 0.86,
                    provenance,
                    provenance_summary: provenance_summary_for_char_refs(&unique_refs),
                    mcids: mcids_for_char_refs(&unique_refs),
                    role: unique_refs
                        .iter()
                        .find_map(|ch| {
                            ch.struct_role
                                .as_ref()
                                .map(|role| text_role_from_tag(role))
                                .filter(|role| *role != TextRole::Unknown)
                        })
                        .unwrap_or(TextRole::Unknown),
                    role_source: unique_refs
                        .iter()
                        .map(|ch| ch.role_source)
                        .find(|source| {
                            matches!(source, TextRoleSource::Tagged | TextRoleSource::RoleMap)
                        })
                        .unwrap_or(TextRoleSource::Unknown),
                    includes_hidden: unique_refs.iter().any(|ch| {
                        ch.provenance
                            .contains(&TextProvenanceFlag::HiddenOrInvisible)
                    }),
                });
            }
            start = to.max(start + 1);
        }
    }
    matches
}

#[derive(Debug, Clone, Copy)]
struct SearchItem<'a> {
    ch: char,
    char_ref: Option<&'a TextSemanticChar>,
}

fn searchable_stream<'a>(
    page: &'a TextSemanticPage,
    options: &TextSearchOptions,
) -> Vec<SearchItem<'a>> {
    let mut raw = Vec::new();
    for block in &page.blocks {
        for line in &block.lines {
            for ch in &line.chars {
                if !options.include_hidden
                    && ch
                        .provenance
                        .contains(&TextProvenanceFlag::HiddenOrInvisible)
                {
                    continue;
                }
                raw.push(SearchItem {
                    ch: ch.text.chars().next().unwrap_or('\u{FFFD}'),
                    char_ref: Some(ch),
                });
            }
            raw.push(SearchItem {
                ch: '\n',
                char_ref: None,
            });
        }
        raw.push(SearchItem {
            ch: '\n',
            char_ref: None,
        });
    }
    normalize_stream(raw, options)
}

fn normalize_query(query: &str, options: &TextSearchOptions) -> String {
    let raw = query
        .chars()
        .map(|ch| SearchItem { ch, char_ref: None })
        .collect();
    normalize_stream(raw, options)
        .into_iter()
        .map(|item| item.ch)
        .collect()
}

fn normalize_stream<'a>(
    raw: Vec<SearchItem<'a>>,
    options: &TextSearchOptions,
) -> Vec<SearchItem<'a>> {
    let mut out = Vec::new();
    let mut idx = 0usize;
    while idx < raw.len() {
        let item = raw[idx];
        if options.ignore_hyphenation
            && item.ch == '-'
            && raw.get(idx + 1).is_some_and(|next| next.ch == '\n')
        {
            idx += 2;
            continue;
        }
        let mut chars = if options.normalize_ligatures {
            ligature_expansion(item.ch)
        } else {
            vec![item.ch]
        };
        for mut ch in chars.drain(..) {
            if options.collapse_whitespace && ch.is_whitespace() {
                ch = ' ';
                if out
                    .last()
                    .is_some_and(|prev: &SearchItem<'_>| prev.ch == ' ')
                {
                    continue;
                }
            }
            if !options.case_sensitive {
                for lower in ch.to_lowercase() {
                    out.push(SearchItem {
                        ch: lower,
                        char_ref: item.char_ref,
                    });
                }
            } else {
                out.push(SearchItem {
                    ch,
                    char_ref: item.char_ref,
                });
            }
        }
        idx += 1;
    }
    out
}

fn ligature_expansion(ch: char) -> Vec<char> {
    match ch {
        '\u{FB00}' => vec!['f', 'f'],
        '\u{FB01}' => vec!['f', 'i'],
        '\u{FB02}' => vec!['f', 'l'],
        '\u{FB03}' => vec!['f', 'f', 'i'],
        '\u{FB04}' => vec!['f', 'f', 'l'],
        '\u{FB05}' => vec!['s', 't'],
        '\u{FB06}' => vec!['s', 't'],
        _ => vec![ch],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(text: &str, x: f64, y: f64, width: f64) -> TextChunk {
        TextChunk {
            text: text.to_string(),
            x,
            y,
            font_size: 10.0,
            font_name: "F1".to_string(),
            width,
            is_rtl: false,
            is_vertical: false,
            is_invisible: false,
            is_actual_text: false,
            mapping_sources: Vec::new(),
        }
    }

    #[test]
    fn semantic_model_builds_words_and_character_quads() {
        let page = build_text_semantic_page(
            1,
            [0.0, 0.0, 200.0, 200.0],
            vec![
                chunk("Hello", 10.0, 150.0, 30.0),
                chunk("world", 44.0, 150.0, 35.0),
            ],
            &TextSemanticOptions::default(),
        );

        assert_eq!(page.counters.words, 2);
        assert_eq!(page.blocks[0].lines[0].words[0].text, "Hello");
        assert_eq!(page.blocks[0].lines[0].words[1].text, "world");
        assert!(page.blocks[0].lines[0].words[0].quad.x1 <= 45.0);
    }

    #[test]
    fn actual_text_and_invisible_provenance_are_reported() {
        let mut actual = chunk("office", 10.0, 100.0, 50.0);
        actual.is_actual_text = true;
        let mut hidden = chunk("ocr", 10.0, 80.0, 20.0);
        hidden.is_invisible = true;
        let page = build_text_semantic_page(
            1,
            [0.0, 0.0, 200.0, 200.0],
            vec![actual, hidden],
            &TextSemanticOptions::default(),
        );

        assert_eq!(page.counters.mapped_via_actual_text, 6);
        assert_eq!(page.counters.hidden_or_invisible, 1);
        assert!(page
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "text.actual_text.used"));
    }

    #[test]
    fn visible_text_mode_excludes_hidden_chunks() {
        let mut hidden = chunk("hidden", 10.0, 100.0, 30.0);
        hidden.is_invisible = true;
        let page = build_text_semantic_page(
            1,
            [0.0, 0.0, 200.0, 200.0],
            vec![chunk("visible", 10.0, 130.0, 40.0), hidden],
            &TextSemanticOptions::visible_text(),
        );

        assert_eq!(page.text(), "visible");
        assert_eq!(page.counters.hidden_or_invisible, 1);
    }

    #[test]
    fn search_matches_ligatures_and_returns_quads() {
        let page = build_text_semantic_page(
            1,
            [0.0, 0.0, 200.0, 200.0],
            vec![chunk("of\u{FB01}ce", 10.0, 100.0, 40.0)],
            &TextSemanticOptions::default(),
        );
        let doc = build_text_semantic_document(vec![page], Vec::new());
        let options = TextSearchOptions {
            case_sensitive: false,
            ..Default::default()
        };

        let matches = doc.search("office", &options);
        assert_eq!(matches.len(), 1);
        assert!(!matches[0].quads.is_empty());
    }

    #[test]
    fn search_matches_hyphenated_line_breaks() {
        let page = build_text_semantic_page(
            1,
            [0.0, 0.0, 200.0, 200.0],
            vec![
                chunk("hyphen-", 10.0, 120.0, 45.0),
                chunk("ated", 10.0, 100.0, 25.0),
            ],
            &TextSemanticOptions::default(),
        );
        let doc = build_text_semantic_document(vec![page], Vec::new());

        let matches = doc.search("hyphenated", &TextSearchOptions::default());
        assert_eq!(matches.len(), 1);
    }

    #[test]
    fn cjk_text_is_tokenized_character_by_character() {
        let page = build_text_semantic_page(
            1,
            [0.0, 0.0, 200.0, 200.0],
            vec![chunk("\u{4F60}\u{597D}", 10.0, 100.0, 20.0)],
            &TextSemanticOptions::default(),
        );

        let words = &page.blocks[0].lines[0].words;
        assert_eq!(words.len(), 2);
        assert_eq!(words[0].text, "\u{4F60}");
        assert_eq!(words[1].text, "\u{597D}");
    }

    #[test]
    fn cjk_dictionary_mode_uses_longest_match_without_rewriting_raw_text() {
        let mut options = TextSemanticOptions {
            cjk_segmentation: CjkSegmentationMode::Dictionary,
            ..TextSemanticOptions::default()
        };
        options.max_cjk_run_chars = 32;
        let page = build_text_semantic_page(
            1,
            [0.0, 0.0, 200.0, 200.0],
            vec![chunk(
                "\u{673A}\u{5668}\u{5B66}\u{4E60}5G\u{691C}\u{7D22}\u{30A8}\u{30F3}\u{30B8}\u{30F3}",
                10.0,
                100.0,
                80.0,
            )],
            &options,
        );

        let line = &page.blocks[0].lines[0];
        assert_eq!(
            line.text,
            "\u{673A}\u{5668}\u{5B66}\u{4E60}5G\u{691C}\u{7D22}\u{30A8}\u{30F3}\u{30B8}\u{30F3}"
        );
        let words: Vec<&str> = line.words.iter().map(|word| word.text.as_str()).collect();
        assert_eq!(
            words,
            vec![
                "\u{673A}\u{5668}\u{5B66}\u{4E60}",
                "5G",
                "\u{691C}\u{7D22}\u{30A8}\u{30F3}\u{30B8}\u{30F3}"
            ]
        );
        assert_eq!(page.counters.cjk_dictionary_tokens, 2);
        assert!(line.words[0]
            .provenance
            .contains(&TextProvenanceFlag::DictionarySegmented));
        let metadata = builtin_cjk_dictionary_metadata();
        assert_eq!(metadata.license, "CC0-1.0 synthetic fixture terms");
        assert!(metadata.entry_count >= 3);
    }
}
