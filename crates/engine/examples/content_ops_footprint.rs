//! Parsed content-operation footprint probe.
//!
//! For every page of a PDF (or the pages given on the command line) this
//! decodes the page content, then measures — with a counting global
//! allocator — what parsing it into `ContentOperation`s costs: wall time,
//! heap allocations, bytes still held by the parsed program, and the peak
//! heap growth during the parse. It then builds the page's retained display
//! list and reports the bytes that list holds.
//!
//! Usage:
//!   cargo run --release -p wellfriendpdf-engine --example content_ops_footprint -- <pdf> [page ...]
//!
//! Output is one tab-separated line per page plus a total line.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use wellfriendpdf_engine::content::ContentParser;
use wellfriendpdf_engine::{ContentEngine, Result};

struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static ALLOCS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            let live = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(live, Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new = unsafe { System.realloc(ptr, layout, new_size) };
        if !new.is_null() {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            if new_size >= layout.size() {
                let grow = new_size - layout.size();
                let live = LIVE.fetch_add(grow, Ordering::Relaxed) + grow;
                PEAK.fetch_max(live, Ordering::Relaxed);
            } else {
                LIVE.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
            }
        }
        new
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

#[derive(Default)]
struct Totals {
    ops: usize,
    content_bytes: usize,
    allocs: usize,
    held: usize,
    peak: usize,
    parse_ns: u128,
    list_held: usize,
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let Some(path) = args.get(1) else {
        eprintln!("usage: content_ops_footprint <pdf> [page ...]");
        std::process::exit(2);
    };
    let engine = ContentEngine::open_bytes(std::fs::read(path)?)?;
    let pages: Vec<usize> = if args.len() > 2 {
        args[2..].iter().filter_map(|value| value.parse().ok()).collect()
    } else {
        (1..=engine.page_count()?).collect()
    };
    println!("page\tops\tcontent_bytes\tparse_allocs\tops_held_bytes\tparse_peak_bytes\tparse_us\tdisplay_list_held_bytes");
    let mut totals = Totals::default();
    let mut max_held = 0usize;
    for page in pages {
        let Ok(content) = engine.document().get_page_content_bytes(page) else {
            continue;
        };
        // Best of three parses for time; allocation figures are identical.
        let mut best_ns = u128::MAX;
        let mut measured = None;
        for _ in 0..3 {
            let base_live = LIVE.load(Ordering::Relaxed);
            PEAK.store(base_live, Ordering::Relaxed);
            let base_allocs = ALLOCS.load(Ordering::Relaxed);
            let start = Instant::now();
            let ops = ContentParser::parse(&content);
            let elapsed = start.elapsed().as_nanos();
            let allocs = ALLOCS.load(Ordering::Relaxed) - base_allocs;
            let held = LIVE.load(Ordering::Relaxed) - base_live;
            let peak = PEAK.load(Ordering::Relaxed) - base_live;
            let count = ops.as_ref().map(Vec::len).unwrap_or(0);
            drop(ops);
            best_ns = best_ns.min(elapsed);
            measured = Some((count, allocs, held, peak));
        }
        let (count, allocs, held, peak) = measured.unwrap_or_default();
        // The engine caches the parsed program while building the list, so
        // the list's own bytes are what dropping it releases.
        let list = engine.build_page_display_list(page, 72);
        let with_list = LIVE.load(Ordering::Relaxed);
        drop(list);
        let list_held = with_list.saturating_sub(LIVE.load(Ordering::Relaxed));
        println!(
            "{page}\t{count}\t{}\t{allocs}\t{held}\t{peak}\t{}\t{list_held}",
            content.len(),
            best_ns / 1000
        );
        totals.ops += count;
        totals.content_bytes += content.len();
        totals.allocs += allocs;
        totals.held += held;
        totals.peak = totals.peak.max(peak);
        totals.parse_ns += best_ns;
        totals.list_held += list_held;
        max_held = max_held.max(held);
    }
    println!(
        "TOTAL\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
        totals.ops,
        totals.content_bytes,
        totals.allocs,
        totals.held,
        totals.peak,
        totals.parse_ns / 1000,
        totals.list_held
    );
    if totals.ops > 0 {
        eprintln!(
            "per op: {:.1} held bytes, {:.2} allocs; max page held {} bytes",
            totals.held as f64 / totals.ops as f64,
            totals.allocs as f64 / totals.ops as f64,
            max_held
        );
    }
    Ok(())
}
