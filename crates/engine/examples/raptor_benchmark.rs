//! Stage-separated RAPTOR benchmark for real PDF corpora.
//!
//! This intentionally runs in one process so source-open, first semantic parse,
//! warm semantic parse, cold raster, warm retained raster, and PNG encoding are
//! not conflated with process startup. It writes one JSON object per PDF to
//! stdout; stderr contains only progress/failure diagnostics.
//!
//! Usage:
//!   cargo run --release -p wellfriendpdf-engine --example raptor_benchmark -- \
//!     <corpus-dir> [dpi=144] [max-files=0]

use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::Serialize;
use sha2::{Digest, Sha256};
use wellfriendpdf_engine::{
    CancelToken, ContentEngine, ExtractionProfile, ImageEncoder, ParseOptions, RenderDocumentCache,
    RenderMode, WellfriendError,
};

#[derive(Serialize)]
struct Observation {
    path: String,
    input_bytes: usize,
    input_sha256: String,
    pages: usize,
    dpi: u32,
    read_ms: f64,
    open_ms: f64,
    page_tree_ms: f64,
    page_program_session_open_ms: f64,
    page_program_parse_cold_ms: f64,
    page_program_parse_warm_ms: f64,
    page_program_operations: usize,
    page_program_output_sha256: String,
    warm_page_program_output_sha256: String,
    page_program_output_exact_match: bool,
    semantic_session_open_ms: f64,
    semantic_parse_cold_ms: f64,
    semantic_parse_warm_ms: f64,
    semantic_output_sha256: String,
    warm_semantic_output_sha256: String,
    semantic_output_exact_match: bool,
    object_cache_hits: u64,
    object_cache_misses: u64,
    object_cache_entries: usize,
    object_cache_estimated_bytes: usize,
    page_artifact_entries: usize,
    page_artifact_estimated_bytes: usize,
    render_page: usize,
    render_session_open_ms: f64,
    raster_cold_ms: f64,
    raster_warm_ms: f64,
    raster_cold_immediate_fallback: bool,
    raster_warm_immediate_fallback: bool,
    png_encode_ms: f64,
    raster_width: u32,
    raster_height: u32,
    raster_sha256: String,
    warm_raster_sha256: String,
    raster_exact_match: bool,
    png_bytes: usize,
}

#[derive(Serialize)]
struct Failure {
    path: String,
    stage: &'static str,
    error: String,
}

fn elapsed_ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn pdfs(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_dir() {
                walk(&path, out)?;
            } else if path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
            {
                out.push(path);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(root, &mut out)?;
    out.sort();
    Ok(out)
}

fn emit_failure(path: &Path, stage: &'static str, error: impl ToString) {
    println!(
        "{}",
        serde_json::to_string(&Failure {
            path: path.display().to_string(),
            stage,
            error: error.to_string(),
        })
        .expect("failure JSON serializes")
    );
}

fn render_with_compat_fallback(
    engine: &ContentEngine,
    page: usize,
    dpi: u32,
    cache: &mut RenderDocumentCache,
) -> wellfriendpdf_engine::Result<(wellfriendpdf_engine::PixelBuffer, bool)> {
    match engine.render_page_cancellable_with_mode_and_cache(
        page,
        dpi,
        &CancelToken::none(),
        RenderMode::Compat,
        cache,
    ) {
        Ok(buffer) => Ok((buffer, false)),
        Err(WellfriendError::UnsupportedFeature(_)) => engine
            .render_page_cancellable_with_mode(page, dpi, &CancelToken::none(), RenderMode::Compat)
            .map(|buffer| (buffer, true)),
        Err(error) => Err(error),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let Some(root) = args.get(1).map(PathBuf::from) else {
        return Err("usage: raptor_benchmark <corpus-dir> [dpi=144] [max-files=0]".into());
    };
    let dpi = args
        .get(2)
        .and_then(|value| value.parse().ok())
        .unwrap_or(144);
    let max_files = args
        .get(3)
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let mut paths = pdfs(&root)?;
    if max_files > 0 {
        paths.truncate(max_files);
    }

    for path in paths {
        let read_start = Instant::now();
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                emit_failure(&path, "read", error);
                continue;
            }
        };
        let read_ms = elapsed_ms(read_start);
        let input_sha256 = digest(&bytes);

        let open_start = Instant::now();
        let structural_engine = match ContentEngine::open_bytes(bytes.clone()) {
            Ok(engine) => engine,
            Err(error) => {
                emit_failure(&path, "open", error);
                continue;
            }
        };
        let open_ms = elapsed_ms(open_start);

        let page_start = Instant::now();
        let pages = match structural_engine.page_count() {
            Ok(pages) if pages > 0 => pages,
            Ok(_) => {
                emit_failure(&path, "page_tree", "document has no pages");
                continue;
            }
            Err(error) => {
                emit_failure(&path, "page_tree", error);
                continue;
            }
        };
        let page_tree_ms = elapsed_ms(page_start);

        // Page-program, document-semantic and rendering sessions are opened
        // independently. That prevents one measured stage from prewarming the
        // immutable artifacts of another and keeps "cold" meaningful.
        let page_program_open_start = Instant::now();
        let page_program_engine = match ContentEngine::open_bytes(bytes.clone()) {
            Ok(engine) => engine,
            Err(error) => {
                emit_failure(&path, "page_program_session_open", error);
                continue;
            }
        };
        let page_program_session_open_ms = elapsed_ms(page_program_open_start);
        let page_program_start = Instant::now();
        let page_program_cold = match page_program_engine.get_page_content(1) {
            Ok(operations) => operations,
            Err(error) => {
                emit_failure(&path, "page_program_parse_cold", error);
                continue;
            }
        };
        let page_program_parse_cold_ms = elapsed_ms(page_program_start);
        let page_program_start = Instant::now();
        let page_program_warm = match page_program_engine.get_page_content(1) {
            Ok(operations) => operations,
            Err(error) => {
                emit_failure(&path, "page_program_parse_warm", error);
                continue;
            }
        };
        let page_program_parse_warm_ms = elapsed_ms(page_program_start);
        let page_program_output_sha256 = digest(format!("{page_program_cold:?}").as_bytes());
        let warm_page_program_output_sha256 = digest(format!("{page_program_warm:?}").as_bytes());

        let semantic_open_start = Instant::now();
        let semantic_engine = match ContentEngine::open_bytes(bytes.clone()) {
            Ok(engine) => engine,
            Err(error) => {
                emit_failure(&path, "semantic_session_open", error);
                continue;
            }
        };
        let semantic_session_open_ms = elapsed_ms(semantic_open_start);
        let parse_options = ParseOptions::default();
        let parse_start = Instant::now();
        let semantic_cold = match semantic_engine
            .parse_document_with_profile(ExtractionProfile::LayoutFaithful, &parse_options)
        {
            Ok(document) => document,
            Err(error) => {
                emit_failure(&path, "semantic_parse_cold", error);
                continue;
            }
        };
        let semantic_parse_cold_ms = elapsed_ms(parse_start);

        let parse_start = Instant::now();
        let semantic_warm = match semantic_engine
            .parse_document_with_profile(ExtractionProfile::LayoutFaithful, &parse_options)
        {
            Ok(document) => document,
            Err(error) => {
                emit_failure(&path, "semantic_parse_warm", error);
                continue;
            }
        };
        let semantic_parse_warm_ms = elapsed_ms(parse_start);
        let semantic_output_sha256 = digest(&serde_json::to_vec(&semantic_cold)?);
        let warm_semantic_output_sha256 = digest(&serde_json::to_vec(&semantic_warm)?);
        let object_cache = semantic_engine.document().reader().object_cache_metrics();
        let page_artifacts = semantic_engine.page_artifact_cache_metrics();

        let render_page = 1;
        let render_open_start = Instant::now();
        let render_engine = match ContentEngine::open_bytes(bytes.clone()) {
            Ok(engine) => engine,
            Err(error) => {
                emit_failure(&path, "render_session_open", error);
                continue;
            }
        };
        let render_session_open_ms = elapsed_ms(render_open_start);
        let mut cache = RenderDocumentCache::new();
        let render_start = Instant::now();
        let (cold, raster_cold_immediate_fallback) =
            match render_with_compat_fallback(&render_engine, render_page, dpi, &mut cache) {
                Ok(result) => result,
                Err(error) => {
                    emit_failure(&path, "raster_cold", error);
                    continue;
                }
            };
        let raster_cold_ms = elapsed_ms(render_start);

        let render_start = Instant::now();
        let (warm, raster_warm_immediate_fallback) =
            match render_with_compat_fallback(&render_engine, render_page, dpi, &mut cache) {
                Ok(result) => result,
                Err(error) => {
                    emit_failure(&path, "raster_warm", error);
                    continue;
                }
            };
        let raster_warm_ms = elapsed_ms(render_start);
        let cold_raw = cold.to_raw_image();
        let warm_raw = warm.to_raw_image();
        let raster_sha256 = digest(&cold_raw.pixels);
        let warm_raster_sha256 = digest(&warm_raw.pixels);

        let encode_start = Instant::now();
        let png = match ImageEncoder::encode_png_fast(&cold_raw) {
            Ok(bytes) => bytes,
            Err(error) => {
                emit_failure(&path, "png_encode", error);
                continue;
            }
        };
        let png_encode_ms = elapsed_ms(encode_start);

        let observation = Observation {
            path: path.display().to_string(),
            input_bytes: bytes.len(),
            input_sha256,
            pages,
            dpi,
            read_ms,
            open_ms,
            page_tree_ms,
            page_program_session_open_ms,
            page_program_parse_cold_ms,
            page_program_parse_warm_ms,
            page_program_operations: page_program_cold.len(),
            page_program_output_sha256: page_program_output_sha256.clone(),
            warm_page_program_output_sha256: warm_page_program_output_sha256.clone(),
            page_program_output_exact_match: page_program_output_sha256
                == warm_page_program_output_sha256,
            semantic_session_open_ms,
            semantic_parse_cold_ms,
            semantic_parse_warm_ms,
            semantic_output_sha256: semantic_output_sha256.clone(),
            warm_semantic_output_sha256: warm_semantic_output_sha256.clone(),
            semantic_output_exact_match: semantic_output_sha256 == warm_semantic_output_sha256,
            object_cache_hits: object_cache.hits,
            object_cache_misses: object_cache.misses,
            object_cache_entries: object_cache.entries,
            object_cache_estimated_bytes: object_cache.estimated_bytes,
            page_artifact_entries: page_artifacts.entries,
            page_artifact_estimated_bytes: page_artifacts.estimated_bytes,
            render_page,
            render_session_open_ms,
            raster_cold_ms,
            raster_warm_ms,
            raster_cold_immediate_fallback,
            raster_warm_immediate_fallback,
            png_encode_ms,
            raster_width: cold.width,
            raster_height: cold.height,
            raster_sha256,
            warm_raster_sha256: warm_raster_sha256.clone(),
            raster_exact_match: warm_raster_sha256 == digest(&cold_raw.pixels),
            png_bytes: png.len(),
        };
        println!("{}", serde_json::to_string(&observation)?);
    }
    Ok(())
}
