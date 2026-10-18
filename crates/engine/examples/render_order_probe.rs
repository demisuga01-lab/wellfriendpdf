//! Render-cache transparency probe.
//!
//! `render_order_probe <pdf> <dpi> order <page,page,...>` renders pages in the
//! given order with one shared `RenderDocumentCache` and compares each with a
//! fresh-cache render.
//!
//! `render_order_probe <pdf> <dpi> bisect <warm_page> <target_page>` renders
//! the target first in a clean process as the reference, then for each cache
//! component warms the cache on `warm_page`, resets every other component and
//! renders the target again, naming the components whose retained state alone
//! changes the target's output. An `all-reset` row checks state outside the
//! cache (thread-local or process-wide).
use wellfriendpdf_engine::{
    render_page_image_with_cache, ContentEngine, RasterImageFormat, RenderDocumentCache,
};

fn render(
    engine: &ContentEngine,
    page: usize,
    dpi: u32,
    cache: &mut RenderDocumentCache,
) -> Vec<u8> {
    let (png, _, _) =
        render_page_image_with_cache(engine, page, dpi, RasterImageFormat::Png, 90, cache)
            .expect("render page");
    png
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let engine = ContentEngine::open_path(&args[1]).expect("open PDF");
    let dpi: u32 = args[2].parse().expect("dpi");
    match args[3].as_str() {
        "order" => {
            let order: Vec<usize> = args[4]
                .split(',')
                .map(|p| p.parse().expect("page"))
                .collect();
            let mut shared = RenderDocumentCache::new();
            for &page in &order {
                let warm = render(&engine, page, dpi, &mut shared);
                let fresh = render(&engine, page, dpi, &mut RenderDocumentCache::new());
                println!(
                    "page {page}: shared {} fresh",
                    if warm == fresh { "==" } else { "!=" }
                );
            }
        }
        "bisect" => {
            let warm_page: usize = args[4].parse().expect("warm page");
            let target: usize = args[5].parse().expect("target page");
            let reference = render(&engine, target, dpi, &mut RenderDocumentCache::new());
            let components = RenderDocumentCache::DIAGNOSTIC_COMPONENTS;
            let mut experiments: Vec<(&str, Vec<&str>)> = vec![("all-reset", components.to_vec())];
            experiments.push(("none-reset", Vec::new()));
            for &keep in components {
                experiments.push((
                    keep,
                    components.iter().copied().filter(|c| *c != keep).collect(),
                ));
            }
            let drops: Vec<String> = components.iter().map(|c| format!("drop:{c}")).collect();
            for (label, &drop) in drops.iter().zip(components) {
                experiments.push((label.as_str(), vec![drop]));
            }
            for (label, resets) in experiments {
                let mut cache = RenderDocumentCache::new();
                render(&engine, warm_page, dpi, &mut cache);
                for component in &resets {
                    assert!(
                        cache.diagnostic_reset(component),
                        "unknown component {component}"
                    );
                }
                let output = render(&engine, target, dpi, &mut cache);
                println!(
                    "keep {label:18} target {}",
                    if output == reference {
                        "same"
                    } else {
                        "DIFFERS"
                    }
                );
            }
        }
        other => panic!("unknown mode {other}"),
    }
}
