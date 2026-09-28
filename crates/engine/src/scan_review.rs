//! Review gates for uncertain reconstruction. OCR scores are not probabilities
//! of layout fidelity. Calibration provenance is caller-supplied, not certified.
use crate::document_subsystems::OcrVisibleReplacement;
use crate::{Result, WellfriendError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecognitionCalibration {
    pub provider_id: String,
    pub provider_version: String,
    pub held_out_dataset_sha256: String,
    pub alpha: f64,
    /// 1 - probability assigned to the verified true reading, on independent
    /// held-out examples. The engine cannot establish exchangeability for you.
    pub true_label_nonconformity: Vec<f64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadingAlternative {
    pub text: String,
    pub probability: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WordReview {
    pub replacement_index: usize,
    pub alternatives: Vec<ReadingAlternative>,
    /// Marks a truncated candidate list. Missing readings can never be treated
    /// as absent simply because the recognizer did not return them.
    pub other_probability: f64,
    #[serde(default)]
    pub texture_or_graphics_intersection: bool,
    #[serde(default)]
    pub handwritten_or_outlined: bool,
    #[serde(default)]
    pub reviewer: Option<String>,
    #[serde(default)]
    pub reviewed_word_sha256: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrReconstructionReview {
    pub input_sha256: String,
    #[serde(default)]
    pub calibration: Option<RecognitionCalibration>,
    pub words: Vec<WordReview>,
    /// Required even for singleton recognition: inpainting reconstructs pixels
    /// and cannot recover arbitrary obscured artwork with certainty.
    pub accept_approximate_background: bool,
}
#[derive(Debug, Clone, Serialize)]
pub struct WordReviewDecision {
    pub replacement_index: usize,
    pub candidate_readings: Vec<String>,
    pub unlisted_readings_possible: bool,
    pub manual_review_required: bool,
    pub manual_review_bound: bool,
}
#[derive(Debug, Clone, Serialize)]
pub struct ScanReviewReport {
    pub decisions: Vec<WordReviewDecision>,
    pub calibration_dataset_sha256: Option<String>,
    pub statistical_scope: String,
    pub background_recovery_guaranteed: bool,
}
pub fn reviewed_word_fingerprint(word: &OcrVisibleReplacement) -> Result<String> {
    let bytes =
        serde_json::to_vec(word).map_err(|e| WellfriendError::invalid_input(e.to_string()))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}
fn threshold(calibration: &RecognitionCalibration) -> Result<f64> {
    let fail = |s: &str| WellfriendError::invalid_input(s);
    let scores = &calibration.true_label_nonconformity;
    if scores.is_empty()
        || scores.len() > 1_000_000
        || !calibration.alpha.is_finite()
        || !(0.0..1.0).contains(&calibration.alpha)
        || calibration.alpha == 0.0
        || scores
            .iter()
            .any(|s| !s.is_finite() || !(0.0..=1.0).contains(s))
        || calibration.held_out_dataset_sha256.len() != 64
        || !calibration
            .held_out_dataset_sha256
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
    {
        return Err(fail("invalid recognition calibration"));
    }
    let rank = (((scores.len() + 1) as f64) * (1.0 - calibration.alpha)).ceil() as usize;
    if rank > scores.len() {
        return Ok(f64::INFINITY);
    }
    let mut sorted = scores.clone();
    sorted.sort_by(f64::total_cmp);
    Ok(sorted[rank.saturating_sub(1)])
}
pub fn evaluate_scan_review(
    input: &[u8],
    provider: &str,
    version: Option<&str>,
    replacements: &[OcrVisibleReplacement],
    review: &OcrReconstructionReview,
) -> Result<ScanReviewReport> {
    let fail = |s: &str| WellfriendError::invalid_input(s);
    if review.input_sha256 != format!("{:x}", Sha256::digest(input))
        || review.words.len() != replacements.len()
        || replacements.len() > 10_000
    {
        return Err(fail("scan review revision or word set mismatch"));
    }
    if !review.accept_approximate_background {
        return Err(fail(
            "scan background reconstruction needs explicit approximation approval",
        ));
    }
    let q = if let Some(c) = &review.calibration {
        if c.provider_id != provider || Some(c.provider_version.as_str()) != version {
            return Err(fail("scan calibration provider/version mismatch"));
        }
        Some(threshold(c)?)
    } else {
        None
    };
    let mut indices = BTreeSet::new();
    let mut decisions = Vec::new();
    for word in &review.words {
        crate::cancel::check_current_cancel("scan uncertainty review")?;
        let target = replacements
            .get(word.replacement_index)
            .ok_or_else(|| fail("scan review word index out of bounds"))?;
        if !indices.insert(word.replacement_index)
            || word.alternatives.len() > 256
            || !word.other_probability.is_finite()
            || !(0.0..=1.0).contains(&word.other_probability)
        {
            return Err(fail("invalid scan review alternatives/index"));
        }
        let mut readings = BTreeSet::new();
        let mut probability = word.other_probability;
        for a in &word.alternatives {
            if a.text.len() > 16_384
                || !readings.insert(&a.text)
                || !a.probability.is_finite()
                || !(0.0..=1.0).contains(&a.probability)
            {
                return Err(fail("invalid scan alternative probability/text"));
            }
            probability += a.probability;
        }
        if (probability - 1.0).abs() > 1e-6 {
            return Err(fail(
                "scan alternative probabilities plus other mass must sum to one",
            ));
        }
        let candidates = word
            .alternatives
            .iter()
            .filter(|a| q.is_none_or(|q| 1.0 - a.probability <= q))
            .map(|a| a.text.clone())
            .collect::<Vec<_>>();
        // Other mass is an upper bound on any unlisted reading's probability.
        let unlisted = q.is_none_or(|q| 1.0 - word.other_probability <= q);
        let required = q.is_none()
            || unlisted
            || candidates.len() != 1
            || candidates.first().is_none_or(|s| s != &target.source_text)
            || word.texture_or_graphics_intersection
            || word.handwritten_or_outlined;
        let bound = word.reviewer.as_ref().is_some_and(|r| !r.trim().is_empty())
            && word.reviewed_word_sha256.as_deref()
                == Some(reviewed_word_fingerprint(target)?.as_str());
        if required && !bound {
            return Err(fail("uncertain scanned word requires a reviewer decision bound to the exact word, geometry and replacement"));
        }
        decisions.push(WordReviewDecision {
            replacement_index: word.replacement_index,
            candidate_readings: candidates,
            unlisted_readings_possible: unlisted,
            manual_review_required: required,
            manual_review_bound: bound,
        });
    }
    decisions.sort_by_key(|d| d.replacement_index);
    Ok(ScanReviewReport { decisions,
        calibration_dataset_sha256: review.calibration.as_ref().map(|c| c.held_out_dataset_sha256.clone()),
        statistical_scope: "split-conformal candidate sets only under valid held-out scores and exchangeability; neither assumption is established by this SDK; no per-document correctness or typography guarantee".into(),
        background_recovery_guaranteed: false })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finite_sample_quantile_uses_n_plus_one_and_can_abstain_on_everything() {
        let mut c = RecognitionCalibration {
            provider_id: "p".into(),
            provider_version: "v".into(),
            held_out_dataset_sha256: "a".repeat(64),
            alpha: 0.1,
            true_label_nonconformity: vec![0.1, 0.2, 0.3],
        };
        assert!(threshold(&c).unwrap().is_infinite());
        c.alpha = 0.5;
        assert_eq!(threshold(&c).unwrap(), 0.2);
    }

    #[test]
    fn uncertain_reading_needs_exact_review_and_edit_changes_invalidate_it() {
        let input = b"revision";
        let mut replacement = OcrVisibleReplacement {
            source_text: "cat".into(),
            replacement_text: "dog".into(),
            rect: [0.0, 0.0, 20.0, 10.0],
            font_size: 10.0,
            confidence: 0.6,
            searchable_text_logical_range: None,
            inpaint_padding_pixels: 2,
        };
        let mut review = OcrReconstructionReview {
            input_sha256: format!("{:x}", Sha256::digest(input)),
            calibration: None,
            accept_approximate_background: true,
            words: vec![WordReview {
                replacement_index: 0,
                alternatives: vec![
                    ReadingAlternative {
                        text: "cat".into(),
                        probability: 0.6,
                    },
                    ReadingAlternative {
                        text: "car".into(),
                        probability: 0.4,
                    },
                ],
                other_probability: 0.0,
                texture_or_graphics_intersection: false,
                handwritten_or_outlined: false,
                reviewer: None,
                reviewed_word_sha256: None,
            }],
        };
        assert!(evaluate_scan_review(input, "p", None, &[replacement.clone()], &review).is_err());
        review.words[0].reviewer = Some("reviewer".into());
        review.words[0].reviewed_word_sha256 =
            Some(reviewed_word_fingerprint(&replacement).unwrap());
        assert!(evaluate_scan_review(input, "p", None, &[replacement.clone()], &review).is_ok());
        replacement.rect[2] = 25.0;
        assert!(evaluate_scan_review(input, "p", None, &[replacement], &review).is_err());
    }
}
