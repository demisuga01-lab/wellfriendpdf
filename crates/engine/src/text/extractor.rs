use rayon::prelude::*;

use super::formatter::{TextFormatOptions, TextFormatter};
use super::reading_order::{ReadingOrderReconstructor, TextLine};
use crate::engine::ContentEngine;
use crate::error::Result;

/// Documents with at least this many pages are extracted in parallel. Below
/// this threshold the rayon fan-out/join overhead outweighs the benefit, so we
/// stay on the simple serial path to avoid regressing small-document latency.
const PARALLEL_PAGE_THRESHOLD: usize = 4;
const DEFAULT_PARALLEL_MEMORY_BUDGET_MB: usize = 1536;
const DEFAULT_PARALLEL_PAGE_BUDGET_MB: usize = 384;

#[derive(Debug, Clone, Default)]
pub struct TextExtractOptions {
    /// Which pages to extract. None = all pages.
    pub pages: Option<Vec<usize>>,

    /// Page marker and formatting options.
    pub format: TextFormatOptions,

    /// Reading-order reconstruction config.
    pub reading_order: ReadingOrderReconstructor,
}

pub struct TextExtractor;

impl Default for TextExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl TextExtractor {
    pub fn new() -> Self {
        TextExtractor
    }

    /// Extract text from all or selected pages of a document.
    ///
    /// Multi-page documents are extracted across rayon worker threads (the
    /// parsed [`ContentEngine`] is shared by immutable reference, so every
    /// thread reads the same parsed document — no per-page reparse). Page
    /// output is reassembled in the original page order regardless of the order
    /// threads finish, so the result is byte-identical to serial extraction. A
    /// page that fails to extract returns an error in both paths. Incomplete
    /// output must not satisfy an editing postcondition as a successful result.
    pub fn extract(&self, engine: &ContentEngine, options: &TextExtractOptions) -> Result<String> {
        let total_pages = engine.page_count()?;
        let requested_pages: Vec<usize> = match &options.pages {
            Some(list) => list.clone(),
            None => (1..=total_pages).collect(),
        };
        // Explicit selections treat an out-of-range page as an empty
        // selection, not as evidence that the PDF is malformed. Filter those
        // requests before dispatch while still propagating every extraction
        // failure on a valid page. This preserves the public probing contract
        // without allowing partial valid-page output to satisfy edit
        // postconditions.
        let page_list: Vec<usize> = requested_pages
            .into_iter()
            .filter(|&page_num| {
                let in_range = page_num > 0 && page_num <= total_pages;
                if !in_range {
                    log::warn!("TextExtractor: page {} out of range, skipping", page_num);
                }
                in_range
            })
            .collect();

        let formatter = TextFormatter::new();

        let cancel = crate::cancel::current_cancel_token();
        // Rayon workers need an explicit scope: thread-local cancellation is
        // otherwise lost at the parallel boundary.
        let format_one = |page_num: usize| -> Result<String> {
            cancel.scope(|| {
                cancel.check("document text extraction")?;
                let (page_n, lines) = self.extract_page(engine, page_num, options)?;
                Ok(formatter.format_page(&lines, page_n, &options.format))
            })
        };

        let parallel_window = bounded_text_parallel_window(page_list.len());
        let page_strings: Vec<String> = if parallel_window >= PARALLEL_PAGE_THRESHOLD {
            let mut out = Vec::with_capacity(page_list.len());
            for chunk in page_list.chunks(parallel_window) {
                // Each chunk is bounded by a conservative memory budget. Rayon
                // preserves input order for `collect()`, and chunks are appended
                // sequentially, so aggregate output remains byte-identical to
                // serial extraction.
                for page in chunk.par_iter().map(|&p| format_one(p)).collect::<Vec<_>>() {
                    out.push(page?);
                }
            }
            out
        } else {
            page_list
                .iter()
                .map(|&p| format_one(p))
                .collect::<Result<Vec<_>>>()?
        };

        let mut all_text = String::new();
        for page_str in page_strings {
            all_text.push_str(&page_str);
        }

        Ok(all_text)
    }

    /// Extract and reconstruct text for a single page.
    pub fn extract_page(
        &self,
        engine: &ContentEngine,
        page_number: usize,
        options: &TextExtractOptions,
    ) -> Result<(usize, Vec<TextLine>)> {
        let chunks = engine.collect_page_text_chunks(page_number)?;

        let lines = options.reading_order.reconstruct(chunks);

        Ok((page_number, lines))
    }

    /// Convenience: extract text from all pages with default options.
    pub fn extract_default(engine: &ContentEngine) -> Result<String> {
        TextExtractor::new().extract(engine, &TextExtractOptions::default())
    }
}

pub fn bounded_text_parallel_window(selected_pages: usize) -> usize {
    if selected_pages < PARALLEL_PAGE_THRESHOLD {
        return 1;
    }
    if cfg!(target_arch = "wasm32") {
        return 1;
    }
    if let Ok(raw) = std::env::var("WELLFRIENDPDF_TEXT_PARALLEL_PAGES") {
        if let Ok(value) = raw.parse::<usize>() {
            return value.max(1).min(selected_pages);
        }
    }

    let workers = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1);
    let memory_budget_mb = std::env::var("WELLFRIENDPDF_TEXT_PARALLEL_MEMORY_MB")
        .ok()
        .and_then(|raw| raw.parse::<usize>().ok())
        .unwrap_or(DEFAULT_PARALLEL_MEMORY_BUDGET_MB)
        .max(1);
    let page_budget_mb = std::env::var("WELLFRIENDPDF_TEXT_PARALLEL_PAGE_MB")
        .ok()
        .and_then(|raw| raw.parse::<usize>().ok())
        .unwrap_or(DEFAULT_PARALLEL_PAGE_BUDGET_MB)
        .max(1);
    let memory_window = (memory_budget_mb / page_budget_mb).max(1);
    workers.min(memory_window).min(selected_pages)
}
