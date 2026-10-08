//! PEBQ adapter for Wellfriend PDF.
//!
//! This mirrors `tools/pebq/native_adapter.cpp` so the same controller can
//! measure a fresh process or a retained process without changing the PDF
//! operation being timed.

use std::fs::File;
use std::io::{self, BufRead, BufWriter, Read, Write};
use std::path::Path;
use std::time::Instant;

use serde_json::{json, Value};
use wellfriendpdf_engine::{
    CancelToken, ContentEngine, PdfDocument, RenderDocumentCache, RenderMode,
};

fn elapsed_ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1_000.0
}

fn read_file(path: &Path) -> io::Result<Vec<u8>> {
    let mut file = File::open(path)?;
    let mut bytes = Vec::with_capacity(
        file.metadata()
            .ok()
            .and_then(|metadata| usize::try_from(metadata.len()).ok())
            .unwrap_or(0),
    );
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .fold(1_469_598_103_934_665_603_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(1_099_511_628_211)
        })
}

fn peak_rss_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status.lines().find_map(|line| {
                let mut fields = line.split_whitespace();
                (fields.next()? == "VmHWM:")
                    .then(|| fields.next()?.parse().ok())
                    .flatten()
            })
        })
        .unwrap_or(0)
}

fn write_ppm(path: &Path, width: u32, height: u32, rgb: &[u8]) -> io::Result<()> {
    let mut output = BufWriter::new(File::create(path)?);
    write!(output, "P6\n{width} {height}\n255\n")?;
    output.write_all(rgb)?;
    output.flush()
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    if values.len().is_multiple_of(2) {
        (values[middle - 1] + values[middle]) / 2.0
    } else {
        values[middle]
    }
}

fn execute(
    profile: &str,
    path: &Path,
    dpi: u32,
    output: Option<&Path>,
    iterations: usize,
) -> Value {
    let request_start = Instant::now();
    let result = (|| -> Result<Value, Box<dyn std::error::Error>> {
        let read_start = Instant::now();
        let bytes = read_file(path)?;
        let read_ms = elapsed_ms(read_start);
        let input_bytes = bytes.len();

        if profile == "page-count" {
            let parse_start = Instant::now();
            let document = PdfDocument::open_bytes(bytes)?;
            let page_count = document.page_count()?;
            let parse_ms = elapsed_ms(parse_start);
            return Ok(json!({
                "engine": "wellfriendpdf",
                "status": "ok",
                "profile": profile,
                "path": path,
                "input_bytes": input_bytes,
                "page_count": page_count,
                "read_ms": read_ms,
                "parse_ms": parse_ms,
                "render_ms": 0.0,
                "write_ms": 0.0,
                "width": 0,
                "height": 0,
                "raster_fnv1a64": "0",
                "peak_rss_kib": peak_rss_kib(),
                "request_ms": elapsed_ms(request_start),
            }));
        }
        if profile != "render" && profile != "render-retained-resources" {
            return Err(format!("unknown profile: {profile}").into());
        }

        let parse_start = Instant::now();
        let engine = ContentEngine::open_bytes(bytes)?;
        let page_count = engine.page_count()?;
        let parse_ms = elapsed_ms(parse_start);

        let retained_resources = profile == "render-retained-resources";
        let sample_count = if retained_resources {
            iterations.max(1)
        } else {
            1
        };
        let mut cache = RenderDocumentCache::resources_only();
        if retained_resources {
            let warmup = engine.render_page_cancellable_with_mode_and_cache(
                1,
                dpi,
                &CancelToken::none(),
                RenderMode::Compat,
                &mut cache,
            )?;
            let _ = warmup.into_raw_image();
        }
        let mut render_samples_ms = Vec::with_capacity(sample_count);
        let mut raster_hashes = Vec::with_capacity(sample_count);
        let mut raw_output = None;
        for _ in 0..sample_count {
            let render_start = Instant::now();
            let raster = if retained_resources {
                engine.render_page_cancellable_with_mode_and_cache(
                    1,
                    dpi,
                    &CancelToken::none(),
                    RenderMode::Compat,
                    &mut cache,
                )?
            } else {
                engine.render_page_cancellable_with_mode(
                    1,
                    dpi,
                    &CancelToken::none(),
                    RenderMode::Compat,
                )?
            };
            let raw = raster.into_raw_image();
            render_samples_ms.push(elapsed_ms(render_start));
            raster_hashes.push(format!("{:x}", fnv1a(&raw.pixels)));
            raw_output = Some(raw);
        }
        let render_ms = median(render_samples_ms.clone());
        let raw = raw_output.ok_or("renderer produced no samples")?;
        let raster_hash = raster_hashes.last().cloned().unwrap_or_default();

        let mut write_ms = 0.0;
        if let Some(output) = output.filter(|path| path.as_os_str() != "-") {
            let write_start = Instant::now();
            write_ppm(output, raw.width, raw.height, &raw.pixels)?;
            write_ms = elapsed_ms(write_start);
        }

        Ok(json!({
            "engine": "wellfriendpdf",
            "status": "ok",
            "profile": profile,
            "path": path,
            "input_bytes": input_bytes,
            "page_count": page_count,
            "read_ms": read_ms,
            "parse_ms": parse_ms,
            "render_ms": render_ms,
            "render_samples_ms": render_samples_ms,
            "render_sample_hashes": raster_hashes,
            "retained_resource_profile": retained_resources,
            "final_raster_cache": "disabled",
            "write_ms": write_ms,
            "width": raw.width,
            "height": raw.height,
            "raster_fnv1a64": raster_hash,
            "cache_telemetry": {
                "glyph_outline": cache.glyph_cache_stats(),
                "glyph_mask": cache.glyph_mask_cache_stats(),
                "path_fill_mask": cache.path_fill_mask_cache_stats(),
                "path_stroke_mask": cache.path_stroke_mask_cache_stats(),
                "path_clip_node": cache.path_clip_node_cache_stats(),
                "font_bytes": cache.font_bytes_cache_stats(),
                "font_resolver": cache.font_resolver_cache_stats(),
                "prepared_text": cache.prepared_text_cache_stats(),
                "display_list": cache.display_list_cache_stats(),
                "render_plan": cache.render_plan_cache_stats(),
                "scaled_image": cache.scaled_image_cache_stats(),
            },
            "peak_rss_kib": peak_rss_kib(),
            "request_ms": elapsed_ms(request_start),
        }))
    })();

    match result {
        Ok(value) => value,
        Err(error) => json!({
            "engine": "wellfriendpdf",
            "status": "error",
            "profile": profile,
            "path": path,
            "error": error.to_string(),
            "request_ms": elapsed_ms(request_start),
        }),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 4 && args[1] == "--request" {
        let profile = &args[2];
        let path = Path::new(&args[3]);
        let dpi = args
            .get(4)
            .and_then(|value| value.parse().ok())
            .unwrap_or(144);
        let output = args.get(5).map(Path::new);
        let iterations = args
            .get(6)
            .and_then(|value| value.parse().ok())
            .unwrap_or(1);
        let value = execute(profile, path, dpi, output, iterations);
        println!("{}", serde_json::to_string(&value)?);
        if value["status"] != "ok" {
            std::process::exit(1);
        }
        return Ok(());
    }
    if args.len() == 2 && args[1] == "--server" {
        let stdin = io::stdin();
        let mut stdout = BufWriter::new(io::stdout().lock());
        for line in stdin.lock().lines() {
            let line = line?;
            let fields: Vec<&str> = line.split('\t').collect();
            let value = if fields.len() < 2 {
                json!({"engine": "wellfriendpdf", "status": "error", "error": "invalid request"})
            } else {
                let dpi = fields
                    .get(2)
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(144);
                let output = fields
                    .get(3)
                    .filter(|value| !value.is_empty())
                    .map(Path::new);
                let iterations = fields
                    .get(4)
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(1);
                execute(fields[0], Path::new(fields[1]), dpi, output, iterations)
            };
            writeln!(stdout, "{}", serde_json::to_string(&value)?)?;
            stdout.flush()?;
        }
        return Ok(());
    }
    Err("usage: pebq_adapter --request <page-count|render|render-retained-resources> <pdf> [dpi] [output.ppm] [iterations] | --server".into())
}
