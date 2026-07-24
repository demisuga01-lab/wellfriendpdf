//! The **OCR boundary**: a pure-Rust interface for turning a page *image* into
//! positioned text, plus the pure-Rust image **preprocessing** that makes that
//! text usable.
//!
//! # The purity contract
//!
//! This module is strictly pure Rust and links no C library. OCR itself is
//! defined only as a [`trait`](OcrEngine) — the parse pipeline depends on the
//! trait, never on a concrete engine. The default backend (Tesseract) lives in
//! the separate, optional `wellfriendpdf-ocr-tesseract` crate and drives the external
//! `tesseract` *process* (no linked C). With no engine injected, scanned pages
//! degrade gracefully to the placeholder the digital-born pipeline already
//! emits (see [`crate::parse`]).
//!
//! # Why this is *not* a second product
//!
//! OCR's only job is to turn page-image pixels into **positioned words** (text +
//! bounding box + confidence) — exactly the shape the digital-born path already
//! recovers from a content stream. Once OCR produces positioned words, they feed
//! the *same* layout → reading-order → table → semantic pipeline
//! ([`crate::docmodel`]); there is no OCR-specific layout/table/semantic code.
//! The seam is [`OcrPage`] → synthetic [`crate::text::TextChunk`]s → the shared
//! page assembly.
//!
//! # The quality lever
//!
//! Tesseract on a raw scan is mediocre; on a **deskewed, binarized, denoised**
//! image at ~300 DPI it is dramatically better. That preprocessing
//! ([`preprocess`]) is pure Rust and lives here, applied before the image ever
//! reaches a backend. It is where scan quality is earned.

pub mod dispatch;
pub mod preprocess;

use crate::error::Result;
use crate::images::decoder::RawImage;

/// When the OCR stage runs, per page. Chosen by the caller; the engine turns it
/// into a per-page decision via [`OcrPolicy::should_ocr`].
///
/// This makes OCR activation an explicit, three-valued policy rather than the
/// implicit "an engine is present ⇒ OCR the scanned pages" it started as:
///
/// - [`OcrPolicy::Off`] — never OCR. Byte-identical to the pre-OCR behavior:
///   scanned pages degrade to the placeholder note + full-page scan figure. This
///   is the default, and is also the effective policy whenever no backend is
///   configured (there is nothing to dispatch to).
/// - [`OcrPolicy::Auto`] — OCR the pages the [`crate::classify`] classifier
///   routed to [`crate::classify::PageSource::Scanned`] (image-only, no usable
///   text layer). Digital-born and searchable-scan pages keep their real text
///   layer and are never re-OCR'd. This is the sensible default when a backend
///   *is* configured.
/// - [`OcrPolicy::Force`] — OCR **every** selected page regardless of the
///   classifier, including digital-born ones. For scans the classifier
///   mislabeled as digital-born (e.g. a thin junk text layer over a scan), or
///   deliberate full re-OCR. Expensive (renders + recognizes every page); the
///   caller opts in knowingly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OcrPolicy {
    /// Never OCR (default). Identical to pre-OCR behavior.
    #[default]
    Off,
    /// OCR only classifier-detected scanned pages.
    Auto,
    /// OCR every selected page, ignoring the classifier.
    Force,
}

impl OcrPolicy {
    /// Whether a page classified as `source` should be OCR'd under this policy.
    /// [`OcrPolicy::Off`] never OCRs; [`OcrPolicy::Force`] always does;
    /// [`OcrPolicy::Auto`] OCRs exactly the scanned pages.
    pub fn should_ocr(self, source: crate::classify::PageSource) -> bool {
        match self {
            OcrPolicy::Off => false,
            OcrPolicy::Force => true,
            OcrPolicy::Auto => source == crate::classify::PageSource::Scanned,
        }
    }

    /// Parse a CLI/config token (`off` / `auto` / `force`, case-insensitive).
    /// Returns `None` for an unrecognized value so the caller can error with a
    /// surface-appropriate message.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" | "none" | "false" => Some(OcrPolicy::Off),
            "auto" | "on" | "true" => Some(OcrPolicy::Auto),
            "force" | "all" => Some(OcrPolicy::Force),
            _ => None,
        }
    }

    /// The token accepted by [`OcrPolicy::parse`] (for round-tripping / help).
    pub fn as_str(self) -> &'static str {
        match self {
            OcrPolicy::Off => "off",
            OcrPolicy::Auto => "auto",
            OcrPolicy::Force => "force",
        }
    }
}

/// A single-channel 8-bit grayscale image — the substrate every preprocessing
/// step and every [`OcrEngine`] backend works on.
///
/// Kept deliberately minimal (no external `image`-crate type) so the core takes
/// on no new dependency: it is just `width × height` luminance bytes, row-major,
/// top-left origin (image/pixel space, y-**down** — the convention OCR engines
/// and the renderer's pixel buffers use).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrImage {
    pub width: u32,
    pub height: u32,
    /// `width * height` luminance bytes, row-major, top-left origin.
    pub gray: Vec<u8>,
}

impl OcrImage {
    /// A blank (all-white) image of the given size.
    pub fn white(width: u32, height: u32) -> Self {
        OcrImage {
            width,
            height,
            gray: vec![255u8; width as usize * height as usize],
        }
    }

    /// `true` when the buffer length matches `width * height` and both dims are
    /// non-zero.
    pub fn is_valid(&self) -> bool {
        self.width > 0
            && self.height > 0
            && self.gray.len() == self.width as usize * self.height as usize
    }

    /// Luminance at `(x, y)`, clamped to the image bounds (`255` / white when out
    /// of range, so sampling past an edge reads as background).
    #[inline]
    pub fn get(&self, x: i64, y: i64) -> u8 {
        if x < 0 || y < 0 || x >= self.width as i64 || y >= self.height as i64 {
            return 255;
        }
        self.gray[y as usize * self.width as usize + x as usize]
    }

    /// Build from a decoded [`RawImage`] by converting to luminance (Rec. 601).
    /// Grayscale stays as-is; RGB/RGBA are weighted; any other channel count
    /// averages the present channels. Alpha is composited over white so a
    /// transparent scan does not read as black.
    pub fn from_raw_image(img: &RawImage) -> Self {
        let w = img.width;
        let h = img.height;
        let n = w as usize * h as usize;
        let ch = img.channels as usize;
        let mut gray = vec![255u8; n];
        if ch == 0 || !img.is_valid() {
            return OcrImage {
                width: w,
                height: h,
                gray,
            };
        }
        for (i, px) in gray.iter_mut().enumerate() {
            let base = i * ch;
            let luma = match ch {
                1 => img.pixels[base] as f32,
                2 => {
                    // gray + alpha
                    let a = img.pixels[base + 1] as f32 / 255.0;
                    let g = img.pixels[base] as f32;
                    g * a + 255.0 * (1.0 - a)
                }
                3 => {
                    let r = img.pixels[base] as f32;
                    let g = img.pixels[base + 1] as f32;
                    let b = img.pixels[base + 2] as f32;
                    0.299 * r + 0.587 * g + 0.114 * b
                }
                4 => {
                    let r = img.pixels[base] as f32;
                    let g = img.pixels[base + 1] as f32;
                    let b = img.pixels[base + 2] as f32;
                    let a = img.pixels[base + 3] as f32 / 255.0;
                    let luma = 0.299 * r + 0.587 * g + 0.114 * b;
                    luma * a + 255.0 * (1.0 - a)
                }
                other => {
                    let sum: u32 = img.pixels[base..base + other]
                        .iter()
                        .map(|&v| v as u32)
                        .sum();
                    sum as f32 / other as f32
                }
            };
            *px = luma.round().clamp(0.0, 255.0) as u8;
        }
        OcrImage {
            width: w,
            height: h,
            gray,
        }
    }
}

impl From<&RawImage> for OcrImage {
    fn from(img: &RawImage) -> Self {
        OcrImage::from_raw_image(img)
    }
}

/// One recognized word: its text, its bounding box **in image-pixel space**
/// (`[x0, y0, x1, y1]`, y-**down**, the same space as the [`OcrImage`] handed to
/// the engine), its 0..1 confidence, and an optional line id (the engine's
/// grouping of words into text lines, which helps the downstream rejoin lines).
#[derive(Debug, Clone, PartialEq)]
pub struct OcrWord {
    pub text: String,
    /// `[x0, y0, x1, y1]` in image-pixel space (y-down).
    pub bbox: [f64; 4],
    /// 0..1 recognition confidence.
    pub confidence: f32,
    /// Engine line grouping, when reported (helps line reassembly). `None` if the
    /// backend does not expose it.
    pub line_id: Option<u32>,
}

/// The result of OCR-ing one page image: the positioned words plus the mean
/// per-word confidence (a quick page-quality signal the caller can threshold to
/// flag an unreliable page).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct OcrPage {
    pub words: Vec<OcrWord>,
    /// Mean of the per-word confidences (0 when there are no words).
    pub mean_confidence: f32,
}

impl OcrPage {
    /// Build a page from words, computing [`OcrPage::mean_confidence`].
    pub fn new(words: Vec<OcrWord>) -> Self {
        let mean = if words.is_empty() {
            0.0
        } else {
            words.iter().map(|w| w.confidence).sum::<f32>() / words.len() as f32
        };
        OcrPage {
            words,
            mean_confidence: mean,
        }
    }
}

/// Options handed to an [`OcrEngine`]. Backend-agnostic; a backend ignores hints
/// it does not support.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrOptions {
    /// Tesseract-style language codes, e.g. `["eng"]` or `["eng", "fra"]`. The
    /// matching language data must be installed for the chosen backend.
    pub languages: Vec<String>,
    /// The DPI the page image was rendered at. OCR works best near 300; this is
    /// passed through for backends that use it (and recorded in provenance).
    pub dpi: u32,
    /// Optional page-segmentation-mode hint (Tesseract `--psm`). `None` lets the
    /// backend choose its default (typically automatic page segmentation).
    pub psm: Option<u32>,
}

impl Default for OcrOptions {
    fn default() -> Self {
        OcrOptions {
            languages: vec!["eng".to_string()],
            dpi: 300,
            psm: None,
        }
    }
}

/// The OCR boundary. A backend turns a (preprocessed) page image into positioned
/// words. The parse pipeline depends only on this trait; concrete backends live
/// outside the core (e.g. `wellfriendpdf-ocr-tesseract`).
///
/// `Send + Sync` so an engine can be shared across the rayon-parallel page work
/// the renderer already uses.
pub trait OcrEngine: Send + Sync {
    /// Recognize text in `image`, returning positioned words in image-pixel
    /// space. Implementations must not panic on a malformed image; return an
    /// `Err` instead so the caller can degrade the page gracefully.
    fn recognize(&self, image: &OcrImage, opts: &OcrOptions) -> Result<OcrPage>;

    /// A short engine identifier recorded in provenance (e.g. `"tesseract"`).
    fn name(&self) -> &str;

    /// The backend version string, when known (recorded in provenance for
    /// reproducibility). `None` if the backend cannot report it.
    fn version(&self) -> Option<String> {
        None
    }

    /// The maximum number of pages this backend tolerates being recognized
    /// **concurrently**. The engine caps its bounded parallel OCR window at this
    /// value, so a backend that is not thread-safe, holds a global lock (a Python
    /// backend under the GIL), or talks to a rate-limited remote API can keep the
    /// engine from oversubscribing it.
    ///
    /// The default is `1` — the safe choice for a backend that has not opted in
    /// to concurrency (it serializes OCR, matching the pre-concurrency behavior).
    /// A local process pool (e.g. Tesseract) can raise this to its comfortable
    /// parallelism (say `4`); a cloud API might return `2` to respect a rate
    /// limit. The engine still clamps the effective window to its own global
    /// bound, so returning a large number never exceeds the engine's limit.
    ///
    /// Must return a value `>= 1`; the engine treats `0` as `1`.
    fn max_concurrency(&self) -> usize {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_rgb_uses_rec601_luma() {
        let img = RawImage {
            width: 1,
            height: 1,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![255, 0, 0], // pure red
        };
        let g = OcrImage::from(&img);
        // 0.299 * 255 ≈ 76
        assert_eq!(g.gray, vec![76]);
    }

    #[test]
    fn from_gray_is_identity() {
        let img = RawImage {
            width: 2,
            height: 1,
            channels: 1,
            bits_per_sample: 8,
            pixels: vec![10, 200],
        };
        let g = OcrImage::from(&img);
        assert_eq!(g.gray, vec![10, 200]);
    }

    #[test]
    fn rgba_composited_over_white() {
        // Fully transparent black must read as white, not black.
        let img = RawImage {
            width: 1,
            height: 1,
            channels: 4,
            bits_per_sample: 8,
            pixels: vec![0, 0, 0, 0],
        };
        let g = OcrImage::from(&img);
        assert_eq!(g.gray, vec![255]);
    }

    #[test]
    fn get_clamps_out_of_bounds_to_white() {
        let g = OcrImage::white(2, 2);
        assert_eq!(g.get(-1, 0), 255);
        assert_eq!(g.get(0, 5), 255);
        assert_eq!(g.get(0, 0), 255);
    }

    #[test]
    fn ocr_page_mean_confidence() {
        let page = OcrPage::new(vec![
            OcrWord {
                text: "a".into(),
                bbox: [0.0, 0.0, 1.0, 1.0],
                confidence: 0.8,
                line_id: Some(0),
            },
            OcrWord {
                text: "b".into(),
                bbox: [0.0, 0.0, 1.0, 1.0],
                confidence: 0.6,
                line_id: Some(0),
            },
        ]);
        assert!((page.mean_confidence - 0.7).abs() < 1e-6);
    }
}
