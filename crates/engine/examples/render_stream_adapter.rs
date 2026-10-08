//! Streams every rendered page as concatenated binary PPM frames.
//!
//! The benchmark controller consumes one frame at a time, computes fidelity
//! metrics, and discards it. This avoids retaining a multi-thousand-page raster
//! corpus or measuring PNG/JPEG encoders as part of renderer time.

use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::Path;
use std::time::Instant;

use wellfriendpdf_engine::{
    CancelToken, ContentEngine, PageRenderer, RenderDocumentCache, RenderMode,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        return Err("usage: render_stream_adapter <pdf> <dpi>".into());
    }
    let path = Path::new(&args[1]);
    let dpi: u32 = args[2].parse()?;
    let mut source = File::open(path)?;
    let mut bytes = Vec::with_capacity(
        source
            .metadata()
            .ok()
            .and_then(|metadata| usize::try_from(metadata.len()).ok())
            .unwrap_or(0),
    );
    source.read_to_end(&mut bytes)?;
    let started = Instant::now();
    let engine = ContentEngine::open_bytes(bytes)?;
    let page_count = engine.page_count()?;
    let mut cache = RenderDocumentCache::resources_only();
    let mut stdout = BufWriter::new(std::io::stdout().lock());
    for page in 1..=page_count {
        let cancel = CancelToken::none();
        let retained =
            PageRenderer::get_or_build_display_list_with_cache(&engine, page, dpi, &mut cache);
        let retained_raster = match retained {
            Ok((list, _)) if list.is_fully_supported() => {
                PageRenderer::render_display_list_cancellable_with_mode_and_cache(
                    &engine,
                    page,
                    dpi,
                    list.as_ref(),
                    &cancel,
                    RenderMode::Compat,
                    &mut cache,
                )
            }
            Ok(_) | Err(_) => Err(wellfriendpdf_engine::WellfriendError::UnsupportedFeature(
                "retained display list unavailable for benchmark adapter".to_string(),
            )),
        };
        let raster = match retained_raster {
            Ok(raster) => raster,
            Err(_) => engine.render_page_cancellable_with_mode_and_cache(
                page,
                dpi,
                &cancel,
                RenderMode::Compat,
                &mut cache,
            )?,
        };
        let raw = raster.to_raw_image();
        writeln!(stdout, "P6\n{} {}\n255", raw.width, raw.height)?;
        stdout.write_all(&raw.pixels)?;
    }
    stdout.flush()?;
    eprintln!(
        "{{\"engine\":\"wellfriend\",\"pages\":{page_count},\"elapsed_ms\":{:.6}}}",
        started.elapsed().as_secs_f64() * 1000.0
    );
    Ok(())
}
