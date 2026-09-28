//! Rebase only the source scalar ranges removed by approved initial OCR
//! captures. No search-based matching, pixel inference or offset replay.
use super::*;
use crate::linked_stories::{StoryFrame, StoryFrameLayout};

#[derive(Default)]
pub(super) struct RemovedText {
    pages: BTreeMap<usize, Vec<[usize; 2]>>,
    operands: usize,
}
impl RemovedText {
    pub(super) fn add(&mut self, page: usize, capture: &OcrCapture) -> Result<()> {
        if capture.form_local {
            return Ok(());
        }
        self.operands = self
            .operands
            .checked_add(capture.logical_ranges.len())
            .ok_or_else(|| fail("story OCR operand count overflow"))?;
        if self.operands > 65_536 {
            return Err(fail("story OCR operand budget exceeded"));
        }
        self.pages
            .entry(page)
            .or_default()
            .extend_from_slice(&capture.logical_ranges);
        Ok(())
    }
    pub(super) fn seal(&mut self, frames: &[StoryFrame]) -> Result<()> {
        for ranges in self.pages.values_mut() {
            ranges.sort_unstable();
            if ranges.iter().any(|r| r[0] > r[1])
                || ranges.windows(2).any(|p| p[0][1] > p[1][0] || p[0] == p[1])
            {
                return Err(fail(
                    "OCR source ranges overlap or one carrier belongs to multiple figures",
                ));
            }
        }
        for frame in frames {
            if frame.owner.is_none() {
                if let Some(ranges) = self.pages.get(&frame.page) {
                    rebase(frame.logical_range, ranges)?;
                }
            }
        }
        Ok(())
    }
    /// Called once after all initial captures have been detached, before any
    /// story text write. Validate expected Unicode on that actual revision.
    pub(super) fn rebind_frames(
        &self,
        input: &[u8],
        frames: &mut [StoryFrameLayout],
    ) -> Result<()> {
        let mut models = BTreeMap::new();
        for layout in frames {
            let frame = &mut layout.frame;
            if frame.owner.is_some() {
                continue;
            }
            let Some(ranges) = self.pages.get(&frame.page) else {
                continue;
            };
            crate::cancel::check_current_cancel("story OCR source-range rebinding")?;
            let rebound = rebase(frame.logical_range, ranges)?;
            if let std::collections::btree_map::Entry::Vacant(e) = models.entry(frame.page) {
                e.insert(crate::advanced_editing::analyze_multi_run_text_range(
                    input, frame.page,
                )?);
            }
            let model = models
                .get(&frame.page)
                .ok_or_else(|| fail("OCR-rebound source page missing"))?;
            if rebound[1] > model.logical_text.chars().count()
                || model
                    .logical_text
                    .chars()
                    .skip(rebound[0])
                    .take(rebound[1] - rebound[0])
                    .collect::<String>()
                    != frame.expected_text
            {
                return Err(fail(
                    "OCR source detachment changed a story frame's expected text",
                ));
            }
            frame.logical_range = rebound;
        }
        Ok(())
    }
}

fn rebase(range: [usize; 2], removed: &[[usize; 2]]) -> Result<[usize; 2]> {
    if range[0] > range[1] {
        return Err(fail("invalid source frame range"));
    }
    let mut before = 0usize;
    for [start, end] in removed {
        if *start == *end {
            continue;
        }
        if (*start < range[1] && range[0] < *end)
            || (range[0] == range[1] && *start < range[0] && range[0] < *end)
        {
            return Err(fail("a source glyph cannot belong both to OCR image movement and story text replacement"));
        }
        if *end <= range[0] {
            before = before
                .checked_add(end - start)
                .ok_or_else(|| fail("OCR range displacement overflow"))?;
        }
    }
    Ok([
        range[0]
            .checked_sub(before)
            .ok_or_else(|| fail("OCR range start underflow"))?,
        range[1]
            .checked_sub(before)
            .ok_or_else(|| fail("OCR range end underflow"))?,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rebase_removes_only_preceding_scalars_and_keeps_empty_boundary_anchors() {
        let removed = [[0, 2], [4, 7], [20, 23]];
        assert_eq!(rebase([7, 10], &removed).unwrap(), [2, 5]);
        assert_eq!(rebase([2, 4], &removed).unwrap(), [0, 2]);
        assert_eq!(rebase([4, 4], &removed).unwrap(), [2, 2]);
        assert_eq!(rebase([7, 7], &removed).unwrap(), [2, 2]);
        assert!(rebase([5, 5], &removed).is_err());
        assert!(rebase([1, 3], &removed).is_err());
        assert!(rebase([3, 8], &removed).is_err());
    }
}
