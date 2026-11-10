//! Renders the XFA pages of a PDF to PNG files.
//!
//! `cargo run --example xfa_render -- <pdf> <out-prefix> [dpi]` writes
//! `<out-prefix>-<n>.png` for every XFA page (dynamic forms: the XFA layout
//! pages; static forms: the PDF pages with the XFA foreground) and
//! `<out-prefix>-pdf-<n>.png` for the PDF's own pages rendered by default.

use wellfriendpdf_engine::{ContentEngine, ImageEncoder, Result, WellfriendError};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let usage = || {
        WellfriendError::MalformedPdf(
            "usage: cargo run --example xfa_render -- <pdf> <out-prefix> [dpi]".to_string(),
        )
    };
    let path = args.next().ok_or_else(usage)?;
    let prefix = args.next().ok_or_else(usage)?;
    let dpi = args
        .next()
        .and_then(|dpi| dpi.parse::<u32>().ok())
        .unwrap_or(72);
    let engine = ContentEngine::open_path(&path)?;
    println!("form type: {:?}", engine.xfa_form_type());
    let pages = engine.xfa_page_count()?;
    println!("xfa pages: {pages}; pdf pages: {}", engine.page_count()?);
    for page in 1..=pages {
        let started = std::time::Instant::now();
        let buffer = engine.render_xfa_page(page, dpi)?;
        let png = ImageEncoder::encode_png_fast(&buffer.into_raw_image())?;
        std::fs::write(format!("{prefix}-{page}.png"), png)?;
        println!("xfa page {page}: {:?}", started.elapsed());
    }
    for page in 1..=engine.page_count()?.min(4) {
        let png = engine.render_page_png_fast(page, dpi)?;
        std::fs::write(format!("{prefix}-pdf-{page}.png"), png)?;
    }
    let unsupported = engine.xfa_render_unsupported();
    if !unsupported.is_empty() {
        println!("unsupported: {unsupported:?}");
    }
    Ok(())
}
