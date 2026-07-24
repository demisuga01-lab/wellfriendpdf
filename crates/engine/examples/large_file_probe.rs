use std::env;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::process;
use std::time::Instant;

use serde_json::json;
use wellfriendpdf_engine::{
    ContentEngine, ImageLocateOptions, ImageLocator, ImageOutputFormat, RenderMode,
    TextExtractOptions, TextExtractor,
};

#[derive(Debug)]
struct Args {
    input: PathBuf,
    operation: String,
    pages: String,
    mode: String,
    dpi: u32,
    output: Option<PathBuf>,
    password: Option<String>,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            input: PathBuf::new(),
            operation: String::new(),
            pages: "all".to_string(),
            mode: "page".to_string(),
            dpi: 72,
            output: None,
            password: None,
        }
    }
}

fn main() {
    let start = Instant::now();
    if let Err(err) = run(start) {
        emit(
            start,
            json!({
                "event": "error",
                "error": err.to_string(),
            }),
        );
        eprintln!("{err}");
        process::exit(1);
    }
}

fn run(start: Instant) -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args()?;
    emit(
        start,
        json!({
            "event": "start",
            "operation": args.operation,
            "input": args.input.display().to_string(),
            "mode": args.mode,
            "pages": args.pages,
            "dpi": args.dpi,
        }),
    );

    let open_started = Instant::now();
    let engine = match &args.password {
        Some(password) => ContentEngine::open_path_with_password(&args.input, password.as_bytes())?,
        None => ContentEngine::open_path(&args.input)?,
    };
    emit(
        start,
        json!({
            "event": "opened",
            "phase_elapsed_ms": open_started.elapsed().as_millis(),
        }),
    );

    match args.operation.as_str() {
        "open" => {}
        "page-count" | "info" => {
            let page_count_started = Instant::now();
            let count = engine.page_count()?;
            emit(
                start,
                json!({
                    "event": "page_count",
                    "pages": count,
                    "phase_elapsed_ms": page_count_started.elapsed().as_millis(),
                }),
            );
        }
        "extract-text" => run_extract_text(start, &engine, &args)?,
        "extract-images" => run_extract_images(start, &engine, &args)?,
        "render" => run_render(start, &engine, &args)?,
        other => {
            return Err(format!(
                "unknown operation '{other}'; use open, page-count, extract-text, extract-images, or render"
            )
            .into());
        }
    }

    emit(
        start,
        json!({
            "event": "done",
            "total_elapsed_ms": start.elapsed().as_millis(),
        }),
    );
    Ok(())
}

fn run_extract_text(
    start: Instant,
    engine: &ContentEngine,
    args: &Args,
) -> Result<(), Box<dyn std::error::Error>> {
    let total = engine.page_count()?;
    let pages = parse_pages(&args.pages, total)?;
    emit(
        start,
        json!({
            "event": "page_plan",
            "total_pages": total,
            "selected_pages": pages.len(),
        }),
    );

    if args.mode == "aggregate" {
        let text = TextExtractor::new().extract(
            engine,
            &TextExtractOptions {
                pages: Some(pages.clone()),
                ..Default::default()
            },
        )?;
        let bytes = text.len();
        if let Some(path) = &args.output {
            std::fs::write(path, text)?;
        }
        emit(
            start,
            json!({
                "event": "aggregate_done",
                "pages_completed": pages.len(),
                "bytes": bytes,
            }),
        );
        return Ok(());
    }

    let mut sink = match &args.output {
        Some(path) => Some(BufWriter::new(File::create(path)?)),
        None => None,
    };
    let mut total_bytes = 0usize;
    for page in pages {
        emit(start, json!({"event": "page_start", "page": page}));
        let text = engine.get_page_text(page)?;
        total_bytes += text.len();
        if let Some(writer) = sink.as_mut() {
            writer.write_all(text.as_bytes())?;
            writer.write_all(b"\n")?;
        }
        emit(
            start,
            json!({
                "event": "page_done",
                "page": page,
                "bytes": text.len(),
                "pages_completed": page,
            }),
        );
    }
    if let Some(writer) = sink.as_mut() {
        writer.flush()?;
    }
    emit(
        start,
        json!({
            "event": "extract_text_done",
            "bytes": total_bytes,
        }),
    );
    Ok(())
}

fn run_extract_images(
    start: Instant,
    engine: &ContentEngine,
    args: &Args,
) -> Result<(), Box<dyn std::error::Error>> {
    let total = engine.page_count()?;
    let pages = parse_pages(&args.pages, total)?;
    emit(
        start,
        json!({
            "event": "page_plan",
            "total_pages": total,
            "selected_pages": pages.len(),
        }),
    );

    if args.mode == "aggregate" {
        let images = ImageLocator::find_all_images(
            engine,
            &ImageLocateOptions {
                pages: Some(pages),
                ..Default::default()
            },
        )?;
        emit(
            start,
            json!({
                "event": "images_located",
                "images": images.len(),
            }),
        );
        let mut total_bytes = 0usize;
        for image in images {
            let bytes =
                engine.extract_image_bytes(&image, ImageOutputFormat::Original, Some(85))?;
            total_bytes += bytes.len();
            emit(
                start,
                json!({
                    "event": "image_done",
                    "page": image.page_number,
                    "bytes": bytes.len(),
                    "object": image.object_number,
                }),
            );
        }
        emit(
            start,
            json!({
                "event": "extract_images_done",
                "bytes": total_bytes,
            }),
        );
        return Ok(());
    }

    let mut total_images = 0usize;
    let mut total_bytes = 0usize;
    for page in pages {
        emit(start, json!({"event": "page_start", "page": page}));
        let images = engine.find_page_images(page)?;
        let mut page_bytes = 0usize;
        for image in &images {
            let bytes = engine.extract_image_bytes(image, ImageOutputFormat::Original, Some(85))?;
            page_bytes += bytes.len();
        }
        total_images += images.len();
        total_bytes += page_bytes;
        emit(
            start,
            json!({
                "event": "page_done",
                "page": page,
                "images": images.len(),
                "bytes": page_bytes,
                "pages_completed": page,
            }),
        );
    }
    emit(
        start,
        json!({
            "event": "extract_images_done",
            "images": total_images,
            "bytes": total_bytes,
        }),
    );
    Ok(())
}

fn run_render(
    start: Instant,
    engine: &ContentEngine,
    args: &Args,
) -> Result<(), Box<dyn std::error::Error>> {
    let total = engine.page_count()?;
    let pages = parse_pages(&args.pages, total)?;
    emit(
        start,
        json!({
            "event": "page_plan",
            "total_pages": total,
            "selected_pages": pages.len(),
        }),
    );

    let mut total_bytes = 0usize;
    for page in pages {
        emit(start, json!({"event": "page_start", "page": page}));
        let bytes = engine.render_page_png_fast_with_mode(page, args.dpi, RenderMode::Compat)?;
        total_bytes += bytes.len();
        emit(
            start,
            json!({
                "event": "page_done",
                "page": page,
                "bytes": bytes.len(),
                "pages_completed": page,
            }),
        );
    }
    emit(
        start,
        json!({
            "event": "render_done",
            "bytes": total_bytes,
        }),
    );
    Ok(())
}

fn parse_args() -> Result<Args, Box<dyn std::error::Error>> {
    let mut args = Args::default();
    let mut iter = env::args().skip(1);
    while let Some(flag) = iter.next() {
        match flag.as_str() {
            "--input" => args.input = PathBuf::from(required_value(&mut iter, "--input")?),
            "--operation" => args.operation = required_value(&mut iter, "--operation")?,
            "--pages" => args.pages = required_value(&mut iter, "--pages")?,
            "--mode" => args.mode = required_value(&mut iter, "--mode")?,
            "--dpi" => args.dpi = required_value(&mut iter, "--dpi")?.parse()?,
            "--output" => args.output = Some(PathBuf::from(required_value(&mut iter, "--output")?)),
            "--password" => args.password = Some(required_value(&mut iter, "--password")?),
            "--help" | "-h" => {
                print_help();
                process::exit(0);
            }
            other => return Err(format!("unknown argument '{other}'").into()),
        }
    }
    if args.input.as_os_str().is_empty() {
        return Err("--input is required".into());
    }
    if args.operation.is_empty() {
        return Err("--operation is required".into());
    }
    if args.mode != "page" && args.mode != "aggregate" {
        return Err("--mode must be page or aggregate".into());
    }
    Ok(args)
}

fn required_value(
    iter: &mut impl Iterator<Item = String>,
    flag: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    iter.next()
        .ok_or_else(|| format!("{flag} requires a value").into())
}

fn parse_pages(spec: &str, total: usize) -> Result<Vec<usize>, Box<dyn std::error::Error>> {
    if spec == "all" {
        return Ok((1..=total).collect());
    }
    let mut out = Vec::new();
    for part in spec.split(',').filter(|part| !part.trim().is_empty()) {
        let part = part.trim();
        if let Some((start, end)) = part.split_once('-') {
            let start: usize = start.trim().parse()?;
            let end: usize = end.trim().parse()?;
            if start == 0 || end < start {
                return Err(format!("invalid page range '{part}'").into());
            }
            for page in start..=end.min(total) {
                out.push(page);
            }
        } else {
            let page: usize = part.parse()?;
            if page == 0 {
                return Err("page numbers are 1-indexed".into());
            }
            if page <= total {
                out.push(page);
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

fn emit(start: Instant, value: serde_json::Value) {
    let mut object = match value {
        serde_json::Value::Object(object) => object,
        _ => serde_json::Map::new(),
    };
    object.insert(
        "elapsed_ms".to_string(),
        serde_json::Value::from(start.elapsed().as_millis() as u64),
    );
    println!("{}", serde_json::Value::Object(object));
    let _ = std::io::stdout().flush();
}

fn print_help() {
    eprintln!(
        "Usage: large_file_probe --input FILE --operation OP [--pages all|1-10] [--mode page|aggregate] [--dpi 72] [--output FILE]"
    );
}
