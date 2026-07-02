//! OCR seam **robustness** tests (no Tesseract required): bounded-window memory
//! and per-page containment.
//!
//! These complement `ocr_seam.rs` (which proves recovered text flows through the
//! shared pipeline) by proving the two guarantees the seam makes about an
//! *untrusted* backend running over a *many-page* scan:
//!
//! 1. **Bounded window.** The engine renders and hands out one page image at a
//!    time; it never rasterizes the whole document into RAM up front. A backend
//!    instrumented to record how many page images are alive inside it at once
//!    sees a peak of exactly one — the memory discipline the 2 GB envelope
//!    depends on.
//! 2. **Containment.** A backend that panics, hangs, or returns garbage on one
//!    page fails only that page (which degrades to the placeholder); the run
//!    completes and every other page still OCRs. A backend panic never crosses
//!    the seam as an unwind.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use oxide_engine::{
    ContentEngine, OcrEngine, OcrImage, OcrOptions, OcrPage, OcrPolicy, OcrWord, ParseOptions,
    SerializeOptions,
};

// ── an N-page pure-scan PDF (no text layer) ──────────────────────────────────

struct PdfBuilder {
    objects: Vec<Vec<u8>>,
}
impl PdfBuilder {
    fn new() -> Self {
        Self {
            objects: Vec::new(),
        }
    }
    fn add(&mut self, body: &str) -> usize {
        self.objects.push(body.as_bytes().to_vec());
        self.objects.len()
    }
    fn add_stream(&mut self, dict_extra: &str, stream: &[u8]) -> usize {
        let mut body =
            format!("<< /Length {} {} >>\nstream\n", stream.len(), dict_extra).into_bytes();
        body.extend_from_slice(stream);
        body.extend_from_slice(b"\nendstream");
        self.objects.push(body);
        self.objects.len()
    }
    fn build(&self) -> Vec<u8> {
        let mut pdf = Vec::new();
        pdf.extend_from_slice(b"%PDF-1.7\n");
        let mut offsets = Vec::new();
        for (i, body) in self.objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            pdf.extend_from_slice(body);
            pdf.extend_from_slice(b"\nendobj\n");
        }
        let xref_start = pdf.len();
        pdf.extend_from_slice(format!("xref\n0 {}\n", offsets.len() + 1).as_bytes());
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        for off in &offsets {
            pdf.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF",
                offsets.len() + 1,
                xref_start
            )
            .as_bytes(),
        );
        pdf
    }
}

/// `n` scanned pages, each a 612×792 page whose only content is a full-page
/// image → every page classifies as `Scanned` and routes to the OCR path.
/// A single shared 1×1 image XObject backs every page (kept tiny on purpose —
/// the engine still *renders* each page to a full raster, which is what the
/// bounded-window test measures).
fn scanned_pages(n: usize) -> Vec<u8> {
    let mut b = PdfBuilder::new();
    b.add("<< /Type /Catalog /Pages 2 0 R >>");
    // Object numbers: 1=catalog, 2=pages, 3=shared content, 4=shared image,
    // then one Page object per page starting at 5.
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 5 + i)).collect();
    b.add(&format!(
        "<< /Type /Pages /Kids [{}] /Count {} >>",
        kids.join(" "),
        n
    ));
    b.add_stream("", b"q 612 0 0 792 0 0 cm /Im0 Do Q\n");
    b.add_stream(
        "/Type /XObject /Subtype /Image /Width 1 /Height 1 \
         /ColorSpace /DeviceGray /BitsPerComponent 8",
        &[0x80],
    );
    for _ in 0..n {
        b.add(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
             /Resources << /XObject << /Im0 4 0 R >> >> /Contents 3 0 R >>",
        );
    }
    b.build()
}

/// One recognized word so the page is treated as recovered (non-empty) text.
fn one_word() -> OcrPage {
    OcrPage::new(vec![OcrWord {
        text: "Recovered".to_string(),
        bbox: [72.0, 60.0, 200.0, 88.0],
        confidence: 0.95,
        line_id: Some(0),
    }])
}

// ── bounded-window: peak concurrent page images inside the backend ───────────

/// Records how many page images are alive *inside* `recognize` simultaneously,
/// tracking the peak. With the engine's one-page-at-a-time discipline the peak
/// must be 1 no matter how many pages the document has.
struct WindowProbe {
    live: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
    calls: Arc<AtomicUsize>,
}

impl OcrEngine for WindowProbe {
    fn recognize(&self, image: &OcrImage, _o: &OcrOptions) -> oxide_engine::Result<OcrPage> {
        assert!(image.is_valid(), "seam handed an invalid image");
        self.calls.fetch_add(1, Ordering::SeqCst);
        let now = self.live.fetch_add(1, Ordering::SeqCst) + 1;
        // Bump the observed peak up to `now`.
        self.peak.fetch_max(now, Ordering::SeqCst);
        // Hold the image briefly so any overlap would be observable.
        std::thread::sleep(Duration::from_millis(2));
        self.live.fetch_sub(1, Ordering::SeqCst);
        Ok(one_word())
    }
    fn name(&self) -> &str {
        "window-probe"
    }
}

#[test]
fn many_page_scan_renders_one_page_image_at_a_time() {
    const PAGES: usize = 40;
    let peak = Arc::new(AtomicUsize::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let probe = WindowProbe {
        live: Arc::new(AtomicUsize::new(0)),
        peak: Arc::clone(&peak),
        calls: Arc::clone(&calls),
    };

    let engine = ContentEngine::open_bytes(scanned_pages(PAGES)).unwrap();
    let opts = ParseOptions {
        ocr: Some(Arc::new(probe)),
        ocr_policy: OcrPolicy::Auto,
        ocr_dpi: 72,
        ..ParseOptions::default()
    };
    let doc = engine.parse_document(&opts).unwrap();

    // Every scanned page was visited exactly once…
    assert_eq!(
        calls.load(Ordering::SeqCst),
        PAGES,
        "each scanned page should be OCR'd exactly once"
    );
    // …and never more than one page image was live in the backend at a time.
    // This is the bounded-window guarantee: no all-pages-rendered blow-up.
    assert_eq!(
        peak.load(Ordering::SeqCst),
        1,
        "engine must hand out one page image at a time (peak concurrency)"
    );
    assert_eq!(doc.pages.len(), PAGES);
}

// ── containment: panic / hang / garbage on one page fails only that page ─────

/// Panics on the 3rd page (1-based), succeeds on every other page.
struct PanicOnPage3 {
    seen: Arc<AtomicUsize>,
}
impl OcrEngine for PanicOnPage3 {
    fn recognize(&self, _i: &OcrImage, _o: &OcrOptions) -> oxide_engine::Result<OcrPage> {
        let n = self.seen.fetch_add(1, Ordering::SeqCst) + 1;
        if n == 3 {
            panic!("simulated backend crash on page 3");
        }
        Ok(one_word())
    }
    fn name(&self) -> &str {
        "panic-on-3"
    }
}

#[test]
fn a_backend_panic_fails_only_its_page_not_the_run() {
    const PAGES: usize = 5;
    let seen = Arc::new(AtomicUsize::new(0));
    let engine = ContentEngine::open_bytes(scanned_pages(PAGES)).unwrap();
    let opts = ParseOptions {
        ocr: Some(Arc::new(PanicOnPage3 {
            seen: Arc::clone(&seen),
        })),
        ocr_policy: OcrPolicy::Auto,
        ocr_dpi: 72,
        ..ParseOptions::default()
    };

    // The run completes — the panic did not cross the seam as an unwind.
    let doc = engine.parse_document(&opts).unwrap();
    assert_eq!(seen.load(Ordering::SeqCst), PAGES, "all pages attempted");
    assert_eq!(doc.pages.len(), PAGES);

    let md = doc.to_markdown(&SerializeOptions::default());
    // Page 3 degraded to the placeholder; the others recovered text.
    assert!(
        md.contains("scanned page 3"),
        "panicking page 3 must degrade to the placeholder:\n{md}"
    );
    assert!(
        md.matches("Recovered").count() >= PAGES - 1,
        "every non-panicking page should still recover text:\n{md}"
    );
}

/// Sleeps far longer than the engine timeout on page 2, succeeds elsewhere.
struct HangOnPage2 {
    seen: Arc<AtomicUsize>,
}
impl OcrEngine for HangOnPage2 {
    fn recognize(&self, _i: &OcrImage, _o: &OcrOptions) -> oxide_engine::Result<OcrPage> {
        let n = self.seen.fetch_add(1, Ordering::SeqCst) + 1;
        if n == 2 {
            std::thread::sleep(Duration::from_secs(30));
        }
        Ok(one_word())
    }
    fn name(&self) -> &str {
        "hang-on-2"
    }
}

#[test]
fn a_hung_backend_is_bounded_by_the_engine_timeout() {
    const PAGES: usize = 3;
    let seen = Arc::new(AtomicUsize::new(0));
    let engine = ContentEngine::open_bytes(scanned_pages(PAGES)).unwrap();
    let opts = ParseOptions {
        ocr: Some(Arc::new(HangOnPage2 {
            seen: Arc::clone(&seen),
        })),
        ocr_policy: OcrPolicy::Auto,
        ocr_dpi: 72,
        ocr_timeout: Some(Duration::from_millis(200)),
        ..ParseOptions::default()
    };

    let start = std::time::Instant::now();
    let doc = engine.parse_document(&opts).unwrap();
    // The whole parse returns promptly — nowhere near the backend's 30s sleep.
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "engine timeout must bound the hung page, got {:?}",
        start.elapsed()
    );

    let md = doc.to_markdown(&SerializeOptions::default());
    assert!(
        md.contains("scanned page 2"),
        "the hung page must degrade to the placeholder:\n{md}"
    );
}

/// Returns a word whose bbox is NaN/huge/degenerate — garbage the merge step
/// must tolerate without panicking.
struct GarbageEngine;
impl OcrEngine for GarbageEngine {
    fn recognize(&self, _i: &OcrImage, _o: &OcrOptions) -> oxide_engine::Result<OcrPage> {
        Ok(OcrPage::new(vec![
            OcrWord {
                text: "ok".to_string(),
                bbox: [10.0, 10.0, 40.0, 24.0],
                confidence: 0.9,
                line_id: Some(0),
            },
            OcrWord {
                text: "nan".to_string(),
                bbox: [f64::NAN, f64::NAN, f64::INFINITY, f64::NAN],
                confidence: 2.5, // out of [0,1]
                line_id: Some(0),
            },
            OcrWord {
                text: "inverted".to_string(),
                bbox: [500.0, 500.0, 10.0, 10.0], // x1<x0, y1<y0
                confidence: -1.0,
                line_id: None,
            },
        ]))
    }
    fn name(&self) -> &str {
        "garbage"
    }
}

#[test]
fn garbage_word_geometry_does_not_crash_the_merge() {
    let engine = ContentEngine::open_bytes(scanned_pages(1)).unwrap();
    let opts = ParseOptions {
        ocr: Some(Arc::new(GarbageEngine)),
        ocr_policy: OcrPolicy::Auto,
        ocr_dpi: 72,
        ..ParseOptions::default()
    };
    // The merge must not panic on NaN/inf/inverted boxes; the page parses.
    let doc = engine.parse_document(&opts).unwrap();
    assert_eq!(doc.pages.len(), 1);
    // The one sane word survives into the output.
    let md = doc.to_markdown(&SerializeOptions::default());
    assert!(md.contains("ok"), "sane word should survive:\n{md}");
}

// ── policy: Off with an engine present is byte-identical to no engine ─────────

#[test]
fn policy_off_with_engine_matches_no_engine() {
    let pdf = scanned_pages(2);

    let plain = ContentEngine::open_bytes(pdf.clone()).unwrap();
    let plain_md = plain
        .parse_document(&ParseOptions::default())
        .unwrap()
        .to_markdown(&SerializeOptions::default());

    let with_engine = ContentEngine::open_bytes(pdf).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let opts = ParseOptions {
        ocr: Some(Arc::new(WindowProbe {
            live: Arc::new(AtomicUsize::new(0)),
            peak: Arc::new(AtomicUsize::new(0)),
            calls: Arc::clone(&calls),
        })),
        ocr_policy: OcrPolicy::Off, // explicitly off despite an engine
        ocr_dpi: 72,
        ..ParseOptions::default()
    };
    let off_md = with_engine
        .parse_document(&opts)
        .unwrap()
        .to_markdown(&SerializeOptions::default());

    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "policy Off must never invoke the backend"
    );
    assert_eq!(
        plain_md, off_md,
        "policy Off with an engine must be byte-identical to no engine"
    );
}
