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
use wellfriendpdf_engine::{CancelToken, ContentEngine, PdfDocument, RenderMode};

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
    bytes.iter().fold(1_469_598_103_934_665_603_u64, |hash, byte| {
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

fn execute(profile: &str, path: &Path, dpi: u32, output: Option<&Path>) -> Value {
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
        if profile != "render" {
            return Err(format!("unknown profile: {profile}").into());
        }

        let parse_start = Instant::now();
        let engine = ContentEngine::open_bytes(bytes)?;
        let page_count = engine.page_count()?;
        let parse_ms = elapsed_ms(parse_start);

        let render_start = Instant::now();
        let raster = engine.render_page_cancellable_with_mode(
            1,
            dpi,
            &CancelToken::none(),
            RenderMode::Compat,
        )?;
        let raw = raster.to_raw_image();
        let render_ms = elapsed_ms(render_start);
        let raster_hash = fnv1a(&raw.pixels);

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
            "write_ms": write_ms,
            "width": raw.width,
            "height": raw.height,
            "raster_fnv1a64": format!("{raster_hash:x}"),
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
        let dpi = args.get(4).and_then(|value| value.parse().ok()).unwrap_or(144);
        let output = args.get(5).map(Path::new);
        let value = execute(profile, path, dpi, output);
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
                let dpi = fields.get(2).and_then(|value| value.parse().ok()).unwrap_or(144);
                let output = fields.get(3).filter(|value| !value.is_empty()).map(Path::new);
                execute(fields[0], Path::new(fields[1]), dpi, output)
            };
            writeln!(stdout, "{}", serde_json::to_string(&value)?)?;
            stdout.flush()?;
        }
        return Ok(());
    }
    Err("usage: pebq_adapter --request <page-count|render> <pdf> [dpi] [output.ppm] | --server".into())
}
