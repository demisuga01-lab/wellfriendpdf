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
    SafeFirstByteChunked,
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

/// Safe accelerated marker candidate discovery.
///
/// The acceleration is deliberately narrow: find offsets whose first byte can
/// begin any requested marker, then validate the full marker at that exact
/// offset. It does not parse lexical context, so comments, strings, names, and
/// binary streams produce the same raw candidates as the scalar reference.
pub fn scan_markers_accelerated(data: &[u8], markers: &[&[u8]]) -> Vec<MarkerCandidate> {
    let mut first_bytes = Vec::new();
    for &marker in markers {
        if let Some(first) = marker.first().copied() {
            if !first_bytes.contains(&first) {
                first_bytes.push(first);
            }
        }
    }
    if first_bytes.is_empty() || data.is_empty() {
        return Vec::new();
    }

    let mut candidates = Vec::new();
    let mut cursor = 0usize;
    while cursor < data.len() {
        let Some(relative) = data[cursor..]
            .iter()
            .position(|byte| first_bytes.contains(byte))
        else {
            break;
        };
        let offset = cursor + relative;
        for &marker in markers {
            if marker.is_empty() {
                continue;
            }
            let end = offset.saturating_add(marker.len());
            if end <= data.len() && &data[offset..end] == marker {
                candidates.push(MarkerCandidate {
                    marker: String::from_utf8_lossy(marker).into_owned(),
                    offset,
                });
            }
        }
        cursor = offset + 1;
    }
    candidates.sort_by(|a, b| {
        a.offset
            .cmp(&b.offset)
            .then_with(|| a.marker.cmp(&b.marker))
    });
    candidates
}

pub fn find_marker_scalar(data: &[u8], marker: &[u8]) -> Option<usize> {
    if marker.is_empty() || marker.len() > data.len() {
        return None;
    }
    data.windows(marker.len())
        .position(|window| window == marker)
}

pub fn find_marker_accelerated(data: &[u8], marker: &[u8]) -> Option<usize> {
    let first = marker.first().copied()?;
    if marker.len() > data.len() {
        return None;
    }
    let mut cursor = 0usize;
    while cursor < data.len() {
        let relative = data[cursor..].iter().position(|byte| *byte == first)?;
        let offset = cursor + relative;
        let end = offset.saturating_add(marker.len());
        if end <= data.len() && &data[offset..end] == marker {
            return Some(offset);
        }
        cursor = offset + 1;
    }
    None
}

pub fn rfind_marker_scalar(data: &[u8], marker: &[u8]) -> Option<usize> {
    if marker.is_empty() || marker.len() > data.len() {
        return None;
    }
    data.windows(marker.len())
        .rposition(|window| window == marker)
}

pub fn rfind_marker_accelerated(data: &[u8], marker: &[u8]) -> Option<usize> {
    if marker.is_empty() || marker.len() > data.len() {
        return None;
    }
    let last_start = data.len() - marker.len();
    (0..=last_start)
        .rev()
        .find(|&offset| &data[offset..offset + marker.len()] == marker)
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
        implementation: ScannerImplementation::SafeFirstByteChunked,
        candidates: scan_markers_accelerated(data, PDF_DELIMITER_MARKERS),
    }
}

pub fn scanner_availability_report() -> serde_json::Value {
    serde_json::json!({
        "default_implementation": "safe_first_byte_chunked",
        "scalar_reference": "available",
        "unsafe_code": false,
        "portable_simd": "not_required",
        "fallback": "scalar reference remains available and tested",
        "lexical_scope": "raw delimiter and stream marker candidates only; parser validates lexical context",
        "markers": PDF_DELIMITER_MARKERS.iter().map(|m| String::from_utf8_lossy(m).to_string()).collect::<Vec<_>>()
    })
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

    #[test]
    fn accelerated_matches_scalar_for_malformed_contexts() {
        let cases: &[&[u8]] = &[
            b"% comment with obj endstream trailer\n1 0 obj\n<<>>\nendobj",
            b"(literal string with stream and endstream) /Name#20withxref",
            b"<656e6473747265616d> /streamName /endobjName",
            b"BI /W 1 /H 1 /CS /RGB ID binary endstream obj EI",
            b"xref\n0 1\n0000000000 65535 f\ntrailer\n<<>>\nstartxref\n0",
            b"endstreamendobjstreamxreftrailerstartxrefobj",
        ];
        for data in cases {
            assert_eq!(
                scan_pdf_markers_scalar(data).candidates,
                scan_pdf_markers_accelerated(data).candidates,
                "case: {}",
                String::from_utf8_lossy(data)
            );
        }
    }

    #[test]
    fn marker_find_helpers_match_scalar() {
        let data = b"aaa startxref bbb xref ccc endstream ddd startxref";
        for marker in [
            b"startxref".as_slice(),
            b"xref".as_slice(),
            b"endstream".as_slice(),
        ] {
            assert_eq!(
                find_marker_scalar(data, marker),
                find_marker_accelerated(data, marker)
            );
            assert_eq!(
                rfind_marker_scalar(data, marker),
                rfind_marker_accelerated(data, marker)
            );
        }
    }

    #[test]
    fn metamorphic_padding_preserves_shifted_candidate_offsets() {
        let base = b"1 0 obj\n<<>>\nstream\nabc\nendstream\nendobj";
        let prefix = b"% padded comment\n   ";
        let mut padded = prefix.to_vec();
        padded.extend_from_slice(base);
        let base_candidates = scan_pdf_markers_accelerated(base).candidates;
        let padded_candidates = scan_pdf_markers_accelerated(&padded).candidates;
        for candidate in base_candidates {
            assert!(padded_candidates.iter().any(|shifted| {
                shifted.marker == candidate.marker
                    && shifted.offset == candidate.offset + prefix.len()
            }));
        }
    }

    #[test]
    fn deterministic_random_parity_smoke() {
        let mut state = 0x5EED_u64;
        let mut data = Vec::new();
        for i in 0..4096 {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            data.push((state >> 32) as u8);
            if i % 257 == 0 {
                data.extend_from_slice(
                    PDF_DELIMITER_MARKERS[(i / 257) % PDF_DELIMITER_MARKERS.len()],
                );
            }
        }
        assert_eq!(
            scan_pdf_markers_scalar(&data).candidates,
            scan_pdf_markers_accelerated(&data).candidates
        );
    }
}
