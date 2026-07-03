use serde::{Deserialize, Serialize};

pub const PDF_DELIMITER_MARKERS: &[&[u8]] = &[
    b"obj",
    b"endobj",
    b"stream",
    b"endstream",
    b"xref",
    b"trailer",
    b"startxref",
];

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MarkerCandidate {
    pub marker: String,
    pub offset: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScannerImplementation {
    Scalar,
    ScalarFallbackNoUnsafeSimd,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MarkerScanResult {
    pub implementation: ScannerImplementation,
    pub candidates: Vec<MarkerCandidate>,
}

/// Correctness reference scanner for PDF delimiter marker candidates.
///
/// This function intentionally returns raw candidates only. Higher-level parser
/// logic still validates lexical context, stream boundaries, and object syntax.
pub fn scan_markers_scalar(data: &[u8], markers: &[&[u8]]) -> Vec<MarkerCandidate> {
    let mut candidates = Vec::new();
    for &marker in markers {
        if marker.is_empty() || marker.len() > data.len() {
            continue;
        }
        for offset in 0..=data.len() - marker.len() {
            if &data[offset..offset + marker.len()] == marker {
                candidates.push(MarkerCandidate {
                    marker: String::from_utf8_lossy(marker).into_owned(),
                    offset,
                });
            }
        }
    }
    candidates.sort_by(|a, b| {
        a.offset
            .cmp(&b.offset)
            .then_with(|| a.marker.cmp(&b.marker))
    });
    candidates
}

pub fn scan_pdf_markers_scalar(data: &[u8]) -> MarkerScanResult {
    MarkerScanResult {
        implementation: ScannerImplementation::Scalar,
        candidates: scan_markers_scalar(data, PDF_DELIMITER_MARKERS),
    }
}

/// Accelerated scanner entry point.
///
/// The engine crate forbids unsafe code, and current stable Rust does not give
/// this crate a portable no-unsafe SIMD primitive. This entry point preserves
/// the abstraction and equality contract while using the scalar implementation.
pub fn scan_pdf_markers_accelerated(data: &[u8]) -> MarkerScanResult {
    MarkerScanResult {
        implementation: ScannerImplementation::ScalarFallbackNoUnsafeSimd,
        candidates: scan_markers_scalar(data, PDF_DELIMITER_MARKERS),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_scanner_finds_expected_markers() {
        let data = b"1 0 obj\n<<>>\nstream\nabc\nendstream\nendobj\nstartxref\n0\n%%EOF";
        let markers = scan_pdf_markers_scalar(data);
        assert!(markers
            .candidates
            .iter()
            .any(|candidate| candidate.marker == "obj" && candidate.offset == 4));
        assert!(markers
            .candidates
            .iter()
            .any(|candidate| candidate.marker == "stream"));
        assert!(markers
            .candidates
            .iter()
            .any(|candidate| candidate.marker == "startxref"));
    }

    #[test]
    fn scanner_reports_raw_false_positive_candidates() {
        let data = b"binary stream bytes with endobj inside image data";
        let markers = scan_pdf_markers_scalar(data);
        assert!(markers
            .candidates
            .iter()
            .any(|candidate| candidate.marker == "endobj"));
    }

    #[test]
    fn accelerated_matches_scalar_exactly() {
        let data = b"obj stream endstream trailer startxref obj endobj xref";
        assert_eq!(
            scan_pdf_markers_scalar(data).candidates,
            scan_pdf_markers_accelerated(data).candidates
        );
    }
}
