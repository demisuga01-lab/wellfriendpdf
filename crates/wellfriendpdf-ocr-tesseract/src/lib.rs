//! Tesseract OCR backend for `wellfriendpdf-engine`.
//!
//! This crate implements [`wellfriendpdf_engine::OcrEngine`] by driving the **external**
//! `tesseract` program as a child process — it links **no C library**. The core
//! engine stays pure Rust and depends only on the [`OcrEngine`] trait; this
//! optional crate is the concrete backend a binary opts into.
//!
//! # How it works
//!
//! For each preprocessed page image (a single-channel [`OcrImage`]) the backend:
//! 1. writes the image to a temporary **PGM** file (a trivial, dependency-free
//!    grayscale format `tesseract` reads natively),
//! 2. invokes `tesseract <in> stdout -l <langs> [--psm N] tsv`, asking for
//!    **TSV** word boxes + confidences (passed as an argument *vector* — never a
//!    shell string — so there is no shell-injection surface),
//! 3. parses the TSV into [`OcrWord`]s (text + pixel bbox + 0..1 confidence +
//!    line id),
//! 4. cleans up the temp file (even on error, via an RAII guard).
//!
//! # Robustness — typed, actionable errors
//!
//! Each failure mode maps to a distinct, actionable [`WellfriendError`], so a caller
//! (or a human reading a log) can tell *why* a page failed:
//!
//! - **binary-not-found** — the `tesseract` program is not on `PATH` (or the
//!   explicit path is wrong). Surfaced at construction as
//!   [`WellfriendError::UnsupportedFeature`] with install guidance; never a panic.
//! - **language-data-missing** — the binary ran but the requested language pack
//!   (`eng.traineddata`, …) is not installed. Detected from tesseract's stderr
//!   and surfaced as [`WellfriendError::UnsupportedFeature`] naming the language and
//!   how to install it — distinct from a generic recognition failure.
//! - **timeout** — the subprocess exceeded the configured deadline; the child is
//!   **killed and reaped** (no zombie / no handle leak) and
//!   [`WellfriendError::Cancelled`] is returned. This is the backend's *own* inner
//!   timeout, in addition to the engine's outer containment backstop.
//! - **nonzero-exit / unparseable output** — any other non-success exit is a
//!   clean [`WellfriendError::ParseError`] carrying tesseract's stderr, so the page
//!   degrades gracefully.
//!
//! # Concurrency
//!
//! Tesseract runs as independent OS processes, so several pages *can* be OCR'd in
//! genuine parallel. [`TesseractEngine::max_concurrency`] therefore returns a
//! real number tied to the host's CPU count (with a sane cap; overridable via
//! [`TesseractEngine::with_max_concurrency`]) rather than the trait's
//! conservative default of `1`. The engine still clamps the effective window to
//! its own global bound, and each page is rendered one-at-a-time upstream, so
//! this raises throughput without breaking the bounded-memory discipline.
//!
//! # Determinism
//!
//! Tesseract is deterministic for a fixed input + version; the engine version is
//! recorded via [`OcrEngine::version`] for reproducibility.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use wellfriendpdf_engine::{
    OcrEngine, OcrImage, OcrOptions, OcrPage, OcrWord, Result, WellfriendError,
};

/// Default per-page OCR subprocess timeout.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// Upper bound on concurrent `tesseract` processes, regardless of core count.
/// Each tesseract process is itself multi-threaded (OpenMP), so running one per
/// core would oversubscribe the CPU; capping keeps the machine responsive while
/// still exploiting real cross-process parallelism.
const MAX_CONCURRENCY_CAP: usize = 8;

/// The Tesseract-backed [`OcrEngine`]. Construct with [`TesseractEngine::new`]
/// (auto-discovers `tesseract` on `PATH`) or [`TesseractEngine::with_path`].
pub struct TesseractEngine {
    /// Path to the `tesseract` executable.
    binary: PathBuf,
    /// Cached version string (from `tesseract --version`), if it could be read.
    version: Option<String>,
    /// Per-invocation timeout.
    timeout: Duration,
    /// How many `tesseract` processes may run concurrently (see
    /// [`OcrEngine::max_concurrency`]).
    max_concurrency: usize,
}

/// A CPU-tied default for [`TesseractEngine::max_concurrency`]: half the
/// available cores (each tesseract process is itself multi-threaded), at least
/// 1, capped at [`MAX_CONCURRENCY_CAP`].
fn default_concurrency() -> usize {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    (cores / 2).clamp(1, MAX_CONCURRENCY_CAP)
}

impl TesseractEngine {
    /// Discover `tesseract` on `PATH` and probe its version. Returns an
    /// actionable error if the binary is not found or not runnable.
    pub fn new() -> Result<Self> {
        Self::with_path("tesseract")
    }

    /// Use an explicit `tesseract` path (or a bare name resolved via `PATH`).
    /// Probes `--version` to confirm the binary is runnable.
    pub fn with_path(path: impl Into<PathBuf>) -> Result<Self> {
        let binary = path.into();
        let version = probe_version(&binary).map_err(|e| {
            WellfriendError::UnsupportedFeature(format!(
                "could not run tesseract at {:?}: {e}. Install Tesseract OCR and its language \
                 data (e.g. `tesseract-ocr` + `tesseract-ocr-eng`) and ensure the `tesseract` \
                 binary is on PATH, or pass an explicit path.",
                binary
            ))
        })?;
        Ok(TesseractEngine {
            binary,
            version: Some(version),
            timeout: DEFAULT_TIMEOUT,
            max_concurrency: default_concurrency(),
        })
    }

    /// Override the per-page subprocess timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Override how many `tesseract` processes may run concurrently. Clamped to
    /// at least 1. Use this to widen (a beefy dedicated OCR box) or narrow (a
    /// shared host) the parallel window; the engine still clamps to its own
    /// global bound on top of this.
    pub fn with_max_concurrency(mut self, n: usize) -> Self {
        self.max_concurrency = n.max(1);
        self
    }

    /// The discovered binary path.
    pub fn binary_path(&self) -> &Path {
        &self.binary
    }
}

impl OcrEngine for TesseractEngine {
    fn recognize(&self, image: &OcrImage, opts: &OcrOptions) -> Result<OcrPage> {
        if !image.is_valid() {
            return Err(WellfriendError::ParseError(
                "OCR image is empty or malformed".to_string(),
            ));
        }

        // Write the gray image to a temp PGM (RAII-cleaned).
        let tmp = TempPgm::write(image)?;

        // Build the argument vector (NO shell — no injection surface).
        let langs = if opts.languages.is_empty() {
            "eng".to_string()
        } else {
            opts.languages.join("+")
        };
        let mut args: Vec<String> = vec![
            tmp.path.to_string_lossy().into_owned(),
            "stdout".to_string(),
            "-l".to_string(),
            langs.clone(),
        ];
        if let Some(psm) = opts.psm {
            args.push("--psm".to_string());
            args.push(psm.to_string());
        }
        // DPI hint helps Tesseract's internal scaling decisions.
        if opts.dpi > 0 {
            args.push("--dpi".to_string());
            args.push(opts.dpi.to_string());
        }
        // The output "configfile": `tsv` emits the word-box TSV we parse.
        args.push("tsv".to_string());

        let stdout = run_with_timeout(&self.binary, &args, self.timeout, &langs)?;
        let words = parse_tsv(&stdout);
        Ok(OcrPage::new(words))
    }

    fn name(&self) -> &str {
        "tesseract"
    }

    fn version(&self) -> Option<String> {
        self.version.clone()
    }

    fn max_concurrency(&self) -> usize {
        self.max_concurrency
    }
}

/// Probe `tesseract --version`, returning the first line's version token.
fn probe_version(binary: &Path) -> std::io::Result<String> {
    let out = Command::new(binary)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;
    // tesseract prints "tesseract v5.5.0..." to stdout (some builds stderr).
    let text = if !out.stdout.is_empty() {
        String::from_utf8_lossy(&out.stdout)
    } else {
        String::from_utf8_lossy(&out.stderr)
    };
    let first = text.lines().next().unwrap_or("").trim();
    let ver = first
        .split_whitespace()
        .nth(1)
        .unwrap_or(first)
        .trim_start_matches('v')
        .to_string();
    if ver.is_empty() {
        Ok("unknown".to_string())
    } else {
        Ok(ver)
    }
}

/// Run `binary args...`, capturing stdout/stderr, killing the child if it
/// exceeds `timeout`. Returns the captured stdout bytes on a zero exit.
///
/// stdout and stderr are drained on dedicated threads so a full pipe buffer can
/// never deadlock the child, while the main thread polls for completion against
/// the deadline. On a non-zero exit the stderr is inspected so a
/// **missing-language-data** failure maps to a distinct, actionable error rather
/// than a generic parse error. `langs` is the resolved language string, used
/// only to make that message specific.
fn run_with_timeout(
    binary: &Path,
    args: &[String],
    timeout: Duration,
    langs: &str,
) -> Result<Vec<u8>> {
    use std::io::Read;

    let mut child = Command::new(binary)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            WellfriendError::UnsupportedFeature(format!(
                "failed to launch tesseract at {binary:?}: {e}"
            ))
        })?;

    // Move the pipe handles onto reader threads so neither can block the child.
    let mut out_pipe = child.stdout.take();
    let mut err_pipe = child.stderr.take();
    let out_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = out_pipe.as_mut() {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    });
    let err_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = err_pipe.as_mut() {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    });

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait()? {
            Some(status) => break status,
            None => {
                if Instant::now() >= deadline {
                    // Kill AND reap: `wait()` after `kill()` reclaims the child
                    // handle so there is no zombie / no leaked OS handle (the
                    // WinError-1450 lesson applied to spawned OCR processes).
                    let _ = child.kill();
                    let _ = child.wait();
                    // Join the drain threads so their pipe handles close too.
                    let _ = out_handle.join();
                    let _ = err_handle.join();
                    return Err(WellfriendError::Cancelled(format!(
                        "tesseract exceeded the {}s OCR timeout",
                        timeout.as_secs()
                    )));
                }
                std::thread::sleep(Duration::from_millis(15));
            }
        }
    };

    let stdout = out_handle.join().unwrap_or_default();
    let stderr = err_handle.join().unwrap_or_default();

    if !status.success() {
        let err = String::from_utf8_lossy(&stderr);
        // Classify a missing-language-data failure distinctly: tesseract prints
        // a recognizable message when a `*.traineddata` pack is absent.
        if is_missing_language_error(&err) {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "tesseract is missing language data for '{langs}'. Install the matching \
                 language pack (e.g. `tesseract-ocr-eng` for English, or set TESSDATA_PREFIX \
                 to a directory containing the `*.traineddata` files). tesseract said: {}",
                err.trim()
            )));
        }
        return Err(WellfriendError::ParseError(format!(
            "tesseract exited with {status}: {}",
            err.trim()
        )));
    }
    Ok(stdout)
}

/// Whether tesseract's stderr indicates a missing/failed-to-load language pack.
/// Tesseract's wording varies across versions; match the stable fragments.
fn is_missing_language_error(stderr: &str) -> bool {
    let s = stderr.to_ascii_lowercase();
    (s.contains("failed loading language") || s.contains("could not initialize tesseract"))
        || (s.contains("tessdata") && s.contains("error"))
        || s.contains("please make sure the tessdata")
        || (s.contains("data") && s.contains("does not exist"))
}

/// Process-local counter so concurrent page OCR (rayon) does not collide on
/// temp filenames. Files are still created with `create_new(true)`; the counter
/// is only a uniqueness aid, not a security boundary.
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// An RAII temp PGM file: written on construction, deleted on drop.
struct TempPgm {
    path: PathBuf,
}

impl TempPgm {
    fn write(image: &OcrImage) -> Result<Self> {
        let pid = std::process::id();
        let mut last_err = None;
        for attempt in 0..64u32 {
            let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let mut path = std::env::temp_dir();
            path.push(format!(
                "wellfriendpdf-ocr-{pid}-{nanos}-{seq}-{attempt}.pgm"
            ));

            let mut f = match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => file,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    last_err = Some(e);
                    continue;
                }
                Err(e) => return Err(WellfriendError::Io(e)),
            };
            // Binary PGM (P5): "P5\n<w> <h>\n255\n" + raw bytes.
            write!(f, "P5\n{} {}\n255\n", image.width, image.height)?;
            f.write_all(&image.gray)?;
            f.flush()?;
            return Ok(TempPgm { path });
        }

        Err(WellfriendError::Io(last_err.unwrap_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "could not allocate a unique temporary PGM path",
            )
        })))
    }
}

impl Drop for TempPgm {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Parse Tesseract TSV output into [`OcrWord`]s.
///
/// TSV columns (header row present): `level page_num block_num par_num line_num
/// word_num left top width height conf text`. We keep only `level == 5` (word)
/// rows with non-empty text and confidence `>= 0`. `conf` is 0..100 → 0..1.
/// `line_id` is a stable per-page line index synthesized from
/// `(block, par, line)`.
fn parse_tsv(bytes: &[u8]) -> Vec<OcrWord> {
    let text = String::from_utf8_lossy(bytes);
    let mut words = Vec::new();
    let mut line_keys: Vec<(i64, i64, i64)> = Vec::new();

    for line in text.lines() {
        // Skip the header (starts with "level").
        if line.starts_with("level") {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 12 {
            continue;
        }
        let level: i64 = cols[0].parse().unwrap_or(-1);
        if level != 5 {
            continue; // not a word row
        }
        let block: i64 = cols[2].parse().unwrap_or(0);
        let par: i64 = cols[3].parse().unwrap_or(0);
        let line_num: i64 = cols[4].parse().unwrap_or(0);
        let left: f64 = cols[6].parse().unwrap_or(0.0);
        let top: f64 = cols[7].parse().unwrap_or(0.0);
        let width: f64 = cols[8].parse().unwrap_or(0.0);
        let height: f64 = cols[9].parse().unwrap_or(0.0);
        let conf: f32 = cols[10].parse().unwrap_or(-1.0);
        let word = cols[11..].join("\t"); // text may itself contain tabs? rare; rejoin defensively

        if conf < 0.0 || word.trim().is_empty() {
            continue;
        }

        // Synthesize a stable per-page line id.
        let key = (block, par, line_num);
        let line_id = match line_keys.iter().position(|k| *k == key) {
            Some(i) => i as u32,
            None => {
                line_keys.push(key);
                (line_keys.len() - 1) as u32
            }
        };

        words.push(OcrWord {
            text: word,
            bbox: [left, top, left + width, top + height],
            confidence: (conf / 100.0).clamp(0.0, 1.0),
            line_id: Some(line_id),
        });
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A representative Tesseract TSV fragment (header + a couple of word rows
    /// plus the non-word level rows it interleaves). Parsing must keep only the
    /// words, decode their boxes/confidence, and group lines.
    const SAMPLE_TSV: &str = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
1\t1\t0\t0\t0\t0\t0\t0\t600\t800\t-1\t\n\
2\t1\t1\t0\t0\t0\t20\t30\t560\t40\t-1\t\n\
3\t1\t1\t1\t0\t0\t20\t30\t560\t40\t-1\t\n\
4\t1\t1\t1\t1\t0\t20\t30\t300\t40\t-1\t\n\
5\t1\t1\t1\t1\t1\t20\t30\t140\t40\t96\tHello\n\
5\t1\t1\t1\t1\t2\t170\t30\t150\t40\t91\tworld\n\
4\t1\t1\t1\t2\t0\t20\t90\t300\t40\t-1\t\n\
5\t1\t1\t1\t2\t1\t20\t90\t120\t40\t-1\t\n\
5\t1\t1\t1\t2\t2\t150\t90\t160\t40\t88\tSecond\n";

    #[test]
    fn tsv_parses_words_boxes_and_confidence() {
        let words = parse_tsv(SAMPLE_TSV.as_bytes());
        assert_eq!(words.len(), 3, "should keep 3 confident word rows");

        assert_eq!(words[0].text, "Hello");
        assert_eq!(words[0].bbox, [20.0, 30.0, 160.0, 70.0]);
        assert!((words[0].confidence - 0.96).abs() < 1e-6);
        assert_eq!(words[0].line_id, Some(0));

        assert_eq!(words[1].text, "world");
        assert_eq!(words[1].line_id, Some(0), "same TSV line groups together");

        assert_eq!(words[2].text, "Second");
        assert_eq!(words[2].line_id, Some(1), "next TSV line is a new line id");
    }

    #[test]
    fn tsv_skips_negative_confidence_and_empty() {
        // A word row with conf -1 (no text recognized) is dropped.
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
5\t1\t1\t1\t1\t1\t0\t0\t10\t10\t-1\t\n\
5\t1\t1\t1\t1\t2\t0\t0\t10\t10\t50\t   \n";
        let words = parse_tsv(tsv.as_bytes());
        assert!(words.is_empty());
    }

    #[test]
    fn missing_binary_is_actionable_error() {
        let err = TesseractEngine::with_path("definitely-not-a-real-binary-xyz")
            .err()
            .expect("should fail to find the binary");
        let msg = err.to_string();
        assert!(
            msg.contains("Install Tesseract") || msg.contains("could not run tesseract"),
            "error should be actionable, got: {msg}"
        );
    }

    #[test]
    fn temp_pgm_is_cleaned_on_drop() {
        let img = OcrImage::white(4, 4);
        let path = {
            let tmp = TempPgm::write(&img).expect("write pgm");
            assert!(tmp.path.exists());
            tmp.path.clone()
        };
        assert!(!path.exists(), "temp PGM must be removed on drop");
    }

    #[test]
    fn missing_language_stderr_is_classified() {
        // Representative stderr fragments across tesseract versions.
        assert!(is_missing_language_error(
            "Error opening data file /usr/share/tessdata/deu.traineddata\n\
             Please make sure the TESSDATA_PREFIX environment variable is set"
        ));
        assert!(is_missing_language_error(
            "Failed loading language 'fra'\nTesseract couldn't load any languages!"
        ));
        assert!(is_missing_language_error("Could not initialize tesseract."));
        // A generic recognition/exit message is NOT a language error.
        assert!(!is_missing_language_error(
            "read_params_file: parameter not found: foo"
        ));
        assert!(!is_missing_language_error(""));
    }

    #[test]
    fn default_concurrency_is_sane() {
        let n = default_concurrency();
        assert!(n >= 1, "must allow at least one process");
        assert!(n <= MAX_CONCURRENCY_CAP, "must respect the cap");
    }

    #[test]
    fn with_max_concurrency_clamps_to_at_least_one() {
        // Build a struct directly (no binary probe) to test the setter's clamp.
        let e = TesseractEngine {
            binary: "tesseract".into(),
            version: None,
            timeout: DEFAULT_TIMEOUT,
            max_concurrency: default_concurrency(),
        }
        .with_max_concurrency(0);
        assert_eq!(e.max_concurrency(), 1, "0 clamps up to 1");
        let e = e.with_max_concurrency(4);
        assert_eq!(e.max_concurrency(), 4);
    }
}
