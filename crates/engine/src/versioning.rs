//! Deterministic document-versioning and deduplication helpers.
//!
//! Prompt 08 needs a small, auditable foundation for detecting unchanged streams
//! across edits and for comparing reconstructed blocks. These helpers are not a
//! compression engine; they are stable sketches and content chunks used by the
//! writer/conversion layer.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentChunk {
    pub offset: usize,
    pub length: usize,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceDedupGroup {
    pub digest: String,
    pub canonical_index: usize,
    pub duplicate_indices: Vec<usize>,
    pub byte_len: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceDedupReport {
    pub input_count: usize,
    pub unique_count: usize,
    pub duplicate_count: usize,
    pub groups: Vec<ResourceDedupGroup>,
}

/// Split bytes into deterministic content-defined chunks using a bounded rolling
/// hash. Chunk boundaries are stable for identical input and capped by
/// `max_size`, which keeps version/dedup scans memory predictable.
pub fn content_defined_chunks(
    data: &[u8],
    min_size: usize,
    avg_size: usize,
    max_size: usize,
) -> Vec<ContentChunk> {
    if data.is_empty() {
        return Vec::new();
    }
    let min_size = min_size.max(64).min(data.len());
    let avg_size = avg_size.max(min_size.next_power_of_two()).max(128);
    let max_size = max_size.max(avg_size).max(min_size).min(data.len().max(1));
    let mask = avg_size.next_power_of_two().saturating_sub(1) as u64;
    let mut chunks = Vec::new();
    let mut start = 0usize;
    let mut roll = 0u64;
    for (idx, byte) in data.iter().enumerate() {
        roll = roll.rotate_left(5) ^ u64::from(*byte).wrapping_mul(0x100_0000_01b3);
        let len = idx + 1 - start;
        let at_boundary = len >= min_size && (roll & mask) == 0;
        if at_boundary || len >= max_size {
            chunks.push(chunk_digest(data, start, idx + 1 - start));
            start = idx + 1;
            roll = 0;
        }
    }
    if start < data.len() {
        chunks.push(chunk_digest(data, start, data.len() - start));
    }
    chunks
}

/// A deterministic SimHash-like text sketch for near-duplicate blocks.
pub fn simhash_text(text: &str) -> u64 {
    let mut weights = [0i32; 64];
    for token in text
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|token| !token.is_empty())
    {
        let digest = Sha256::digest(token.to_ascii_lowercase().as_bytes());
        let mut bits = [0u8; 8];
        bits.copy_from_slice(&digest[..8]);
        let value = u64::from_be_bytes(bits);
        for (idx, weight) in weights.iter_mut().enumerate() {
            if (value >> idx) & 1 == 1 {
                *weight += 1;
            } else {
                *weight -= 1;
            }
        }
    }
    weights.iter().enumerate().fold(0u64, |acc, (idx, weight)| {
        if *weight >= 0 {
            acc | (1u64 << idx)
        } else {
            acc
        }
    })
}

pub fn hamming_distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

pub fn resource_digest(data: &[u8]) -> String {
    hex_digest(data)
}

/// Group byte-identical resources by SHA-256 digest in deterministic input
/// order. The writer can use this report to avoid emitting duplicate resources
/// when object semantics allow it; the helper itself does not rewrite objects.
pub fn resource_dedup_report(resources: &[Vec<u8>]) -> ResourceDedupReport {
    let mut seen: BTreeMap<String, (usize, usize, Vec<usize>)> = BTreeMap::new();
    for (idx, resource) in resources.iter().enumerate() {
        let digest = resource_digest(resource);
        seen.entry(digest)
            .and_modify(|(_, _, duplicates)| duplicates.push(idx))
            .or_insert((idx, resource.len(), Vec::new()));
    }
    let mut groups = seen
        .into_iter()
        .map(
            |(digest, (canonical_index, byte_len, duplicate_indices))| ResourceDedupGroup {
                digest,
                canonical_index,
                duplicate_indices,
                byte_len,
            },
        )
        .collect::<Vec<_>>();
    groups.sort_by_key(|group| group.canonical_index);
    let duplicate_count = groups
        .iter()
        .map(|group| group.duplicate_indices.len())
        .sum();
    ResourceDedupReport {
        input_count: resources.len(),
        unique_count: groups.len(),
        duplicate_count,
        groups,
    }
}

fn chunk_digest(data: &[u8], offset: usize, length: usize) -> ContentChunk {
    ContentChunk {
        offset,
        length,
        sha256: hex_digest(&data[offset..offset + length]),
    }
}

fn hex_digest(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_defined_chunks_are_deterministic_and_cover_input() {
        let data = b"alpha beta gamma ".repeat(128);
        let a = content_defined_chunks(&data, 128, 256, 512);
        let b = content_defined_chunks(&data, 128, 256, 512);
        assert_eq!(a, b);
        assert_eq!(
            a.iter().map(|chunk| chunk.length).sum::<usize>(),
            data.len()
        );
        assert_eq!(a.first().unwrap().offset, 0);
    }

    #[test]
    fn simhash_detects_near_duplicate_text() {
        let a = simhash_text("invoice total amount due paid by customer");
        let b = simhash_text("invoice total amount due paid by client");
        let c = simhash_text("unrelated raster image color profile overprint");
        assert!(hamming_distance(a, b) < hamming_distance(a, c));
    }

    #[test]
    fn resource_digest_is_stable_hex() {
        let digest = resource_digest(b"resource");
        assert_eq!(digest.len(), 64);
        assert_eq!(digest, resource_digest(b"resource"));
    }

    #[test]
    fn resource_dedup_report_groups_identical_resources() {
        let resources = vec![b"image-a".to_vec(), b"font".to_vec(), b"image-a".to_vec()];
        let report = resource_dedup_report(&resources);
        assert_eq!(report.input_count, 3);
        assert_eq!(report.unique_count, 2);
        assert_eq!(report.duplicate_count, 1);
        let first = &report.groups[0];
        assert_eq!(first.canonical_index, 0);
        assert_eq!(first.duplicate_indices, vec![2]);
    }
}
